//! Walks `TypedProgram` (already fully typed — no type errors possible
//! at this stage) and emits bytecode for spg-vm.
//!
//! Two passes, mirroring spg-typecheck's own approach:
//! 1. collect every function's name -> chunk index, so forward calls
//!    (fn A calls fn B declared below it) resolve correctly, exactly
//!    like `collect_function_signatures` in the checker.
//! 2. compile each function body into its own Chunk; top-level
//!    statements (anything not inside a FunctionDecl) go into `main`.
//!
//! Locals are resolved to numeric slots via a scope stack of
//! `HashMap<String, usize>` per active block, the same block-scoping
//! model the checker uses (own block + nested blocks see the variable).
//! `And`/`Or` compile to short-circuiting jumps rather than an eager
//! bytecode op, since a naive AddOp-style opcode would evaluate both
//! sides even when the left side already decides the result.

use std::collections::HashMap;

use spg_parser::ast::{BinaryOp, UnaryOp};
use spg_typecheck::typed_ast::{RangeStep, Ty, TypedBlock, TypedExpr, TypedProgram, TypedStmt};
use spg_vm::chunk::{Chunk, OpCode, Program};
use spg_vm::value::Value;

use crate::error::CompileError;

/// Compiles with the superinstruction pass enabled (what every normal run uses).
pub fn compile(program: TypedProgram) -> Result<Program, CompileError> {
    compile_with(program, true)
}

/// `optimize == false` skips the peephole pass entirely (`spg --no-opt`): the
/// plain one-op-per-operation bytecode, useful to debug the optimizer and as
/// the reference its output must match.
pub fn compile_with(program: TypedProgram, optimize: bool) -> Result<Program, CompileError> {
    // pass 1: register every top-level function under a stable index
    // ordering matches declaration order, same as the checker's table
    let mut function_names: Vec<String> = Vec::new();
    for stmt in &program.stmts {
        if let TypedStmt::FunctionDecl { name, .. } = stmt {
            function_names.push(name.clone());
        }
    }
    let name_to_index: HashMap<String, usize> =
        function_names.iter().cloned().enumerate().map(|(i, n)| (n, i)).collect();

    // pass 1b: register every struct under a stable index too, plus a
    // per-struct field-name -> position map so MakeStruct/FieldGet/
    // FieldSet can resolve `def_index`/`field_index` at compile time —
    // the VM itself never looks a struct or field up by name
    let mut struct_defs: Vec<std::rc::Rc<spg_vm::value::StructDef>> = Vec::new();
    let mut struct_name_to_index: HashMap<String, usize> = HashMap::new();
    let mut struct_field_indices: HashMap<String, HashMap<String, usize>> = HashMap::new();
    let mut struct_field_types: HashMap<String, HashMap<String, Ty>> = HashMap::new();
    for stmt in &program.stmts {
        if let TypedStmt::StructDecl { name, fields, .. } = stmt {
            let field_names: Vec<String> = fields.iter().map(|p| p.name.clone()).collect();
            let field_indices: HashMap<String, usize> =
                field_names.iter().cloned().enumerate().map(|(i, n)| (n, i)).collect();
            let field_types: HashMap<String, Ty> = fields
                .iter()
                .map(|p| (p.name.clone(), spg_typecheck::resolve_type_expr(&p.type_ann)))
                .collect();
            struct_name_to_index.insert(name.clone(), struct_defs.len());
            struct_field_indices.insert(name.clone(), field_indices);
            struct_field_types.insert(name.clone(), field_types);
            struct_defs.push(std::rc::Rc::new(spg_vm::value::StructDef { name: name.clone(), field_names }));
        }
    }
    // pass 1b': every enum under a stable index, so `Color.Green` compiles to
    // `Value::Enum(index_of_Color, 1)`
    let mut enum_defs: Vec<std::rc::Rc<spg_vm::value::EnumDef>> = Vec::new();
    let mut enum_name_to_index: HashMap<String, u32> = HashMap::new();
    for stmt in &program.stmts {
        if let TypedStmt::EnumDecl { name, variants, values, .. } = stmt {
            enum_name_to_index.insert(name.clone(), enum_defs.len() as u32);
            enum_defs.push(std::rc::Rc::new(spg_vm::value::EnumDef {
                name: spg_parser::ast::display_name(name).to_string(),
                variants: variants.clone(),
                values: values
                    .iter()
                    .map(|v| match v {
                        spg_parser::ast::EnumValue::Int(n) => Value::Int(*n),
                        spg_parser::ast::EnumValue::Float(x) => Value::Float(*x),
                        spg_parser::ast::EnumValue::Str(s) => Value::String(std::rc::Rc::new(s.clone())),
                        spg_parser::ast::EnumValue::Bool(b) => Value::Bool(*b),
                    })
                    .collect(),
            }));
        }
    }
    // pass 1c: every root-scope `let`/`const` becomes a GLOBAL slot.
    // Collected up front (like functions/structs) so a function body
    // compiled before the declaration in source order still resolves
    // it — the typechecker already accepts that, so the compiler must.
    let mut global_slots: HashMap<String, usize> = HashMap::new();
    for stmt in &program.stmts {
        if let TypedStmt::Let { name, .. } | TypedStmt::Const { name, .. } = stmt {
            let next = global_slots.len();
            global_slots.entry(name.clone()).or_insert(next);
        }
    }

    let struct_table = StructTable {
        name_to_index: struct_name_to_index,
        field_indices: struct_field_indices,
        field_types: struct_field_types,
        enum_indices: enum_name_to_index,
        enum_values: enum_defs.iter().map(|d| d.values.clone()).collect(),
    };

    // pass 2: compile each function body, and collect top-level
    // (non-function) statements into `main`
    let mut functions: Vec<Chunk> = Vec::new();

    // one FunctionCompiler lives across every top-level (non-function)
    // statement, so `let`/`const` locals declared by an earlier
    // top-level statement stay resolvable by a later one — mirrors the
    // checker's single top-level scope for `main`
    let sink = std::rc::Rc::new(std::cell::RefCell::new(LambdaSink { base: function_names.len(), chunks: Vec::new() }));
    let mut main_fc = FunctionCompiler::new("main", &name_to_index, &struct_table, &global_slots, true);
    main_fc.sink = sink.clone();
    main_fc.begin_scope();

    for (i, stmt) in program.stmts.iter().enumerate() {
        let loc = to_loc(&program.locs[i]);
        main_fc.chunk.cur_loc = loc.clone();
        match stmt {
            TypedStmt::FunctionDecl { name, params, return_type, body, .. } => {
                let mut fc = FunctionCompiler::new(name.clone(), &name_to_index, &struct_table, &global_slots, false);
                fc.sink = sink.clone();
                fc.chunk.cur_loc = loc;
                fc.chunk.arity = params.len();
                fc.chunk.native_sig = {
                    use spg_parser::ast::TypeExpr;
                    use spg_vm::chunk::NativeKind;
                    let elem = |t: &TypeExpr| match t {
                        TypeExpr::Int => Some(spg_vm::chunk::ElemKind::Int),
                        TypeExpr::Float => Some(spg_vm::chunk::ElemKind::Float),
                        TypeExpr::Bool => Some(spg_vm::chunk::ElemKind::Bool),
                        TypeExpr::String => Some(spg_vm::chunk::ElemKind::String),
                        _ => None,
                    };
                    let kind_t = |t: &TypeExpr| match t {
                        TypeExpr::Int => Some(NativeKind::Int),
                        TypeExpr::Float => Some(NativeKind::Float),
                        TypeExpr::Bool => Some(NativeKind::Bool),
                        TypeExpr::String => Some(NativeKind::String),
                        TypeExpr::Array(inner) => elem(inner).map(NativeKind::Array),
                        _ => None,
                    };
                    let kind_r = |t: &Ty| match t {
                        Ty::Int => Some(NativeKind::Int),
                        Ty::Float => Some(NativeKind::Float),
                        Ty::Bool => Some(NativeKind::Bool),
                        Ty::String => Some(NativeKind::String),
                        _ => None,
                    };
                    let ps: Option<Vec<NativeKind>> = params.iter().map(|p| kind_t(&p.type_ann)).collect();
                    match (ps, kind_r(return_type)) {
                        (Some(ps), Some(r)) => Some((ps, r)),
                        _ => None,
                    }
                };
                fc.begin_scope();
                for p in params {
                    fc.declare_local(&p.name);
                }
                fc.compile_block(body)?;
                // a function whose body doesn't end in an explicit
                // Return (e.g. `-> void` falling off the end, which
                // the typechecker allows for Void returns) needs an
                // implicit return so the VM always has a value to pop
                fc.emit_implicit_return(return_type);
                fc.end_scope();
                functions.push(fc.chunk);
            }
            // struct declarations carry no bytecode of their own — the
            // struct table built above (pass 1b) is what MakeStruct/
            // FieldGet/FieldSet actually consult; imports are resolved
            // before compilation reaches this stage
            TypedStmt::StructDecl { .. } | TypedStmt::EnumDecl { .. } | TypedStmt::Import { .. } => {}
            other => {
                main_fc.compile_stmt(other)?;
            }
        }
    }

    main_fc.end_scope();
    let mut main = main_fc.chunk;
    main.emit(OpCode::PushVoid);
    main.emit(OpCode::Return);

    // the lambdas come after the named functions, in the order they were reserved
    for chunk in sink.borrow_mut().chunks.drain(..) {
        functions.push(chunk);
        function_names.push("<lambda>".to_string());
    }

    // fold common instruction pairs into single opcodes as a post-pass
    // over every already-compiled chunk (see spg-compiler::peephole)
    if optimize {
        crate::peephole::fuse_superinstructions(&mut main);
        for f in &mut functions {
            crate::peephole::fuse_superinstructions(f);
        }
    }

    Ok(Program { main, functions, function_names, struct_defs, enum_defs, num_globals: global_slots.len() })
}

/// where a resolved variable name actually lives
enum VarSlot {
    Local(usize),
    Global(usize),
}

/// struct name/field-name resolution shared read-only by every
/// `FunctionCompiler` — built once in `compile`'s pass 1b, since structs
/// (unlike locals) are visible from every function body regardless of
/// declaration order, the same reason `name_to_index` (functions) is
/// built once and shared the same way.
struct StructTable {
    name_to_index: HashMap<String, usize>,
    /// struct name -> (field name -> positional index)
    field_indices: HashMap<String, HashMap<String, usize>>,
    /// struct name -> (field name -> declared type) — the compiler needs
    /// a field's type (not just its position) to pick the right
    /// arithmetic opcode for `obj.field OP= value`, the same way
    /// IndexCompoundAssign needs the array's element type
    field_types: HashMap<String, HashMap<String, Ty>>,
    /// enum name -> its index in `Program.enum_defs` (the first field of `Value::Enum`)
    enum_indices: HashMap<String, u32>,
    /// per enum (indexed like `Program.enum_defs`): each variant's value
    enum_values: Vec<Vec<Value>>,
}

/// the bodies of every lambda in the program, shared by all `FunctionCompiler`s.
/// A lambda's function index is `base` (the number of named functions) plus its
/// position here, so indices stay stable while compilation is still going
#[derive(Default)]
struct LambdaSink {
    base: usize,
    chunks: Vec<Chunk>,
}

struct FunctionCompiler<'a> {
    chunk: Chunk,
    /// where the lambdas found in this body go (shared with every other compiler)
    sink: std::rc::Rc<std::cell::RefCell<LambdaSink>>,
    /// stack of block scopes; each maps a local variable name to its
    /// slot index in the (flat, per-function) locals array
    scopes: Vec<HashMap<String, usize>>,
    /// next free local slot — only grows, never reused across sibling
    /// blocks, which keeps slot resolution simple at the cost of using
    /// slightly more stack space than strictly necessary
    next_slot: usize,
    function_index: &'a HashMap<String, usize>,
    struct_table: &'a StructTable,
    /// name -> global slot for every root-scope variable of the program
    globals: &'a HashMap<String, usize>,
    /// true only for the `main` chunk. Its outermost scope (depth 1) is
    /// the file's root scope, whose declarations are globals; a function
    /// body's outermost scope is just its own locals.
    is_main: bool,
    /// one entry per while/for-in loop we're currently nested inside
    /// (innermost last). `break`/`continue` always target the top of
    /// this stack — the typechecker already guarantees this stack is
    /// non-empty whenever a Break/Continue node is reached.
    loop_stack: Vec<LoopContext>,
    /// `try` bodies being compiled right now (handlers armed at this point)
    open_handlers: usize,
}

/// bookkeeping for one active loop while its body is being compiled.
/// Both `Jump` instruction indices need their target patched once the
/// loop finishes emitting — `break`s to `after the loop`, `continue`s
/// to `the loop's own "next iteration" point`, which for `while` is
/// the condition re-check and for the desugared `for-in` is the index
/// increment (jumping straight to the condition would skip it and
/// spin forever on the same element).
struct LoopContext {
    break_jumps: Vec<usize>,
    continue_jumps: Vec<usize>,
    /// how many `try` handlers were armed when the loop started: a `break` /
    /// `continue` that leaves a `try` inside the body has to disarm the extra ones
    handlers_at_entry: usize,
}

impl<'a> FunctionCompiler<'a> {
    fn new(
        name: impl Into<String>,
        function_index: &'a HashMap<String, usize>,
        struct_table: &'a StructTable,
        globals: &'a HashMap<String, usize>,
        is_main: bool,
    ) -> Self {
        FunctionCompiler {
            chunk: Chunk::new(name),
            sink: Default::default(),
            scopes: Vec::new(),
            next_slot: 0,
            function_index,
            struct_table,
            globals,
            is_main,
            loop_stack: Vec::new(),
            open_handlers: 0,
        }
    }

    /// `break` / `continue` jump out of any `try` body between them and the loop:
    /// the handlers those `try`s armed must not outlive the jump (a later `throw`
    /// would land in a catch block that is no longer active)
    fn disarm_handlers_to_loop(&mut self) {
        let at_entry = self.loop_stack.last().map_or(0, |l| l.handlers_at_entry);
        for _ in at_entry..self.open_handlers {
            self.chunk.emit(OpCode::PopHandler);
        }
    }

    fn begin_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn end_scope(&mut self) {
        self.scopes.pop();
        // note: next_slot is deliberately NOT rolled back here. Reusing
        // slots across sibling blocks would require the VM to clear
        // stale values on scope exit (else a shadowed-then-reused slot
        // could leak the previous block's last value); v1.0 trades a
        // few extra locals-array entries for that simplicity.
    }

    fn declare_local(&mut self, name: &str) -> usize {
        let slot = self.next_slot;
        self.next_slot += 1;
        self.chunk.num_locals = self.chunk.num_locals.max(self.next_slot);
        self.scopes.last_mut().expect("at least one scope active").insert(name.to_string(), slot);
        slot
    }

    /// true when a declaration made right now lands in the file's root
    /// scope — only possible in `main`, at scope depth 1 (depth 0 means
    /// no scope is open yet; anything deeper is a nested block, whose
    /// variables stay ordinary locals)
    fn at_root_scope(&self) -> bool {
        self.is_main && self.scopes.len() == 1
    }

    /// which storage a name resolves to. Locals shadow globals, so a
    /// function parameter (or a `let` inside a block) named like a
    /// global still wins — the same shadowing the typechecker applies.
    fn resolve_var(&self, name: &str) -> Option<VarSlot> {
        if let Some(slot) = self.resolve_local(name) {
            return Some(VarSlot::Local(slot));
        }
        self.globals.get(name).map(|s| VarSlot::Global(*s))
    }

    fn resolve_local(&self, name: &str) -> Option<usize> {
        for scope in self.scopes.iter().rev() {
            if let Some(slot) = scope.get(name) {
                return Some(*slot);
            }
        }
        None
    }

    /// resolves `object_ty.field`'s positional slot via `struct_table` —
    /// `object_ty` is always `Ty::Named(struct_name)` here since
    /// spg-typecheck already rejected any FieldAssign/FieldCompoundAssign
    /// whose object isn't a real struct, so the lookups below are
    /// expected to always succeed for a program that passed typecheck
    fn resolve_field_index(&self, object_ty: &Ty, field: &str) -> Result<usize, CompileError> {
        let struct_name = match object_ty {
            Ty::Named(n) => n,
            other => {
                return Err(CompileError::InvalidControlFlow(format!(
                    "compiler bug: field access on non-struct type {:?} (should've been caught by typecheck)",
                    other
                )))
            }
        };
        self.struct_table
            .field_indices
            .get(struct_name)
            .and_then(|fields| fields.get(field))
            .copied()
            .ok_or_else(|| {
                CompileError::InvalidControlFlow(format!(
                    "compiler bug: struct '{}' has no field '{}' (should've been caught by typecheck)",
                    struct_name, field
                ))
            })
    }

    /// resolves `object_ty.field`'s declared type via `struct_table` —
    /// needed by FieldCompoundAssign to pick the right arithmetic opcode
    /// (int vs float), the same role `elem_ty` plays for
    /// IndexCompoundAssign
    fn field_type(&self, object_ty: &Ty, field: &str) -> Result<Ty, CompileError> {
        let struct_name = match object_ty {
            Ty::Named(n) => n,
            other => {
                return Err(CompileError::InvalidControlFlow(format!(
                    "compiler bug: field access on non-struct type {:?} (should've been caught by typecheck)",
                    other
                )))
            }
        };
        self.struct_table.field_types.get(struct_name).and_then(|fields| fields.get(field)).cloned().ok_or_else(|| {
            CompileError::InvalidControlFlow(format!(
                "compiler bug: struct '{}' has no field '{}' (should've been caught by typecheck)",
                struct_name, field
            ))
        })
    }

    fn emit_implicit_return(&mut self, return_type: &Ty) {
        match return_type {
            Ty::Void => {
                self.chunk.emit(OpCode::PushVoid);
                self.chunk.emit(OpCode::Return);
            }
            // non-void functions are guaranteed by spg-typecheck's
            // always_returns analysis to return on every path, so this
            // is only ever reached for Void — nothing else to emit
            _ => {}
        }
    }

    // Statements

    fn compile_block(&mut self, block: &TypedBlock) -> Result<(), CompileError> {
        self.begin_scope();
        for (i, stmt) in block.stmts.iter().enumerate() {
            self.chunk.cur_loc = to_loc(&block.locs[i]);
            let is_last = i == block.stmts.len() - 1;
            // the last statement of a block used as an if-expression
            // branch is an expression whose value must be LEFT on the
            // stack (it's the branch's result); every other statement
            // position pops its expression result to keep the stack
            // balanced. block.ty is Void for plain (non-if-expr) blocks.
            if is_last && block.ty != Ty::Void {
                if let TypedStmt::ExprStmt(expr) = stmt {
                    self.compile_expr(expr)?;
                    self.end_scope();
                    return Ok(());
                }
            }
            self.compile_stmt(stmt)?;
        }
        self.end_scope();
        Ok(())
    }

    fn compile_stmt(&mut self, stmt: &TypedStmt) -> Result<(), CompileError> {
        match stmt {
            TypedStmt::Let { name, value, .. } | TypedStmt::Const { name, value, .. } => {
                self.compile_expr(value)?;
                if self.at_root_scope() {
                    // root-scope declaration -> global slot (pre-assigned
                    // in pass 1c); nothing to declare locally
                    let slot = *self
                        .globals
                        .get(name)
                        .expect("compiler bug: root-scope variable missing from the globals table");
                    self.chunk.emit(OpCode::SetGlobal(slot));
                } else {
                    let slot = self.declare_local(name);
                    self.chunk.emit(OpCode::SetLocal(slot));
                }
                self.chunk.emit(OpCode::Pop);
            }

            TypedStmt::Assign { name, value } => {
                self.compile_expr(value)?;
                match self.resolve_var(name).expect(
                    "compiler bug: assign target not found in any scope (should've been caught by typecheck)",
                ) {
                    VarSlot::Local(slot) => self.chunk.emit(OpCode::SetLocal(slot)),
                    VarSlot::Global(slot) => self.chunk.emit(OpCode::SetGlobal(slot)),
                };
                self.chunk.emit(OpCode::Pop);
            }

            TypedStmt::IndexAssign { array, index, value } => {
                // push order matches OpCode::IndexSet's expected pop
                // order (array, then index, then value)
                self.compile_expr(array)?;
                self.compile_expr(index)?;
                self.compile_expr(value)?;
                // a map is indexed by key, with its own opcodes (the fused array-index
                // superinstructions never see a map)
                self.chunk.emit(if matches!(array.ty(), Ty::Map(..)) { OpCode::MapSet } else { OpCode::IndexSet });
            }

            TypedStmt::IndexCompoundAssign { array, index, op, value } => {
                // array/index are pushed exactly once and then Dup2'd —
                // see OpCode::Dup2 and Stmt::IndexCompoundAssign's doc
                // comments for why re-evaluating either would be wrong
                self.compile_expr(array)?;
                self.compile_expr(index)?;
                self.chunk.emit(OpCode::Dup2);
                let is_map = matches!(array.ty(), Ty::Map(..));
                self.chunk.emit(if is_map { OpCode::MapGet } else { OpCode::IndexGet });
                self.compile_expr(value)?;
                // the element type (not the array type) is the left
                // operand type the arithmetic opcode needs to pick
                // int-vs-float arithmetic correctly
                let elem_ty = match array.ty() {
                    Ty::Array(inner) => (**inner).clone(),
                    Ty::Map(_, value_ty) => (**value_ty).clone(),
                    other => {
                        return Err(CompileError::InvalidControlFlow(format!(
                            "compiler bug: IndexCompoundAssign on non-array type {:?} (should've been caught by typecheck)",
                            other
                        )))
                    }
                };
                let opcode = binary_opcode(op, &elem_ty);
                self.chunk.emit(opcode);
                self.chunk.emit(if is_map { OpCode::MapSet } else { OpCode::IndexSet });
            }

            TypedStmt::FieldAssign { object, field, value } => {
                // push order matches OpCode::FieldSet's expected pop
                // order (struct, then value)
                self.compile_expr(object)?;
                self.compile_expr(value)?;
                let field_index = self.resolve_field_index(object.ty(), field)?;
                self.chunk.emit(OpCode::FieldSet { field_index });
            }

            TypedStmt::FieldCompoundAssign { object, field, op, value } => {
                // object is pushed exactly once and then Dup1'd — see
                // OpCode::Dup1 and Stmt::FieldCompoundAssign's doc
                // comments for why re-evaluating it would be wrong
                self.compile_expr(object)?;
                let field_index = self.resolve_field_index(object.ty(), field)?;
                self.chunk.emit(OpCode::Dup1);
                self.chunk.emit(OpCode::FieldGet { field_index });
                self.compile_expr(value)?;
                // the field's type (not the struct's type) is the left
                // operand type the arithmetic opcode needs
                let field_ty = self.field_type(object.ty(), field)?;
                let opcode = binary_opcode(op, &field_ty);
                self.chunk.emit(opcode);
                self.chunk.emit(OpCode::FieldSet { field_index });
            }

            TypedStmt::IfStmt { cond, then_branch, else_branch } => {
                self.compile_expr(cond)?;
                let jump_if_false = self.chunk.emit(OpCode::JumpIfFalse(0));
                self.compile_block(then_branch)?;

                if let Some(else_b) = else_branch {
                    let jump_over_else = self.chunk.emit(OpCode::Jump(0));
                    let else_start = self.chunk.next_index();
                    self.chunk.patch_jump(jump_if_false, else_start);
                    self.compile_block(else_b)?;
                    let after = self.chunk.next_index();
                    self.chunk.patch_jump(jump_over_else, after);
                } else {
                    let after = self.chunk.next_index();
                    self.chunk.patch_jump(jump_if_false, after);
                }
            }

            TypedStmt::Block { body } => {
                self.compile_block(body)?;
            }

            TypedStmt::While { cond, body } => {
                let loop_start = self.chunk.next_index();
                self.compile_expr(cond)?;
                let jump_if_false = self.chunk.emit(OpCode::JumpIfFalse(0));

                self.loop_stack.push(LoopContext { break_jumps: Vec::new(), continue_jumps: Vec::new(), handlers_at_entry: self.open_handlers });
                self.compile_block(body)?;
                // continue targets the condition re-check, i.e. `loop_start`
                // — that's where control needs to go to re-evaluate `cond`
                // before deciding whether to run the body again
                let loop_ctx = self.loop_stack.pop().expect("pushed right above");
                for continue_idx in &loop_ctx.continue_jumps {
                    self.chunk.patch_jump(*continue_idx, loop_start);
                }

                self.chunk.emit(OpCode::Jump(loop_start));
                let after = self.chunk.next_index();
                self.chunk.patch_jump(jump_if_false, after);
                for break_idx in &loop_ctx.break_jumps {
                    self.chunk.patch_jump(*break_idx, after);
                }
            }

            TypedStmt::ForIn { var_name, iterable, body, .. } => {
                self.compile_for_in(var_name, iterable, body)?;
            }

            TypedStmt::ForRange { var_name, start, end, step, var_assigned, body } => {
                self.compile_for_range(var_name, start, end, step, *var_assigned, body)?;
            }

            TypedStmt::Return { value } => {
                match value {
                    Some(expr) => self.compile_expr(expr)?,
                    None => {
                        self.chunk.emit(OpCode::PushVoid);
                    }
                }
                self.chunk.emit(OpCode::Return);
            }

            TypedStmt::Try { try_block, catch_block, .. } => {
                // PushHandler's target is backpatched to the catch
                // block's start; PopHandler after the try body disarms
                // it once the try body finished without throwing
                let push_handler = self.chunk.emit(OpCode::PushHandler { catch_target: 0 });
                self.open_handlers += 1;
                self.compile_block(try_block)?;
                self.open_handlers -= 1;
                self.chunk.emit(OpCode::PopHandler);
                let jump_over_catch = self.chunk.emit(OpCode::Jump(0));

                let catch_start = self.chunk.next_index();
                self.chunk.patch_jump(push_handler, catch_start);
                // the thrown value is left on the stack by the VM's
                // unwind logic when it lands here; bind it to catch_var
                self.begin_scope();
                let slot = self.declare_local(&catch_var_name(stmt));
                self.chunk.emit(OpCode::SetLocal(slot));
                self.chunk.emit(OpCode::Pop);
                self.compile_block(catch_block)?;
                self.end_scope();

                let after = self.chunk.next_index();
                self.chunk.patch_jump(jump_over_catch, after);
            }

            TypedStmt::Throw { value } => {
                self.compile_expr(value)?;
                self.chunk.emit(OpCode::Throw);
            }

            TypedStmt::Break => {
                self.disarm_handlers_to_loop();
                let jump = self.chunk.emit(OpCode::Jump(0));
                self.loop_stack
                    .last_mut()
                    .expect("compiler bug: break outside a loop (should've been caught by typecheck)")
                    .break_jumps
                    .push(jump);
            }

            TypedStmt::Continue => {
                self.disarm_handlers_to_loop();
                let jump = self.chunk.emit(OpCode::Jump(0));
                self.loop_stack
                    .last_mut()
                    .expect("compiler bug: continue outside a loop (should've been caught by typecheck)")
                    .continue_jumps
                    .push(jump);
            }

            // nested function declarations don't exist in v1.0 (per the
            // checker's own single global function table), so this
            // path is unreachable for well-typed programs
            TypedStmt::FunctionDecl { .. } => {}
            TypedStmt::StructDecl { .. } => {}
            TypedStmt::EnumDecl { .. } => {}
            TypedStmt::Import { .. } => {}

            TypedStmt::ExprStmt(expr) => {
                self.compile_expr(expr)?;
                self.chunk.emit(OpCode::Pop);
            }
        }
        Ok(())
    }

    /// desugars `for (x in arr) { body }` into an index-counter while
    /// loop over a hidden local holding the array and one holding the
    /// running index — keeps the VM from needing a dedicated iterator
    /// concept for v1.0, since arrays are the only iterable
    fn compile_for_in(
        &mut self,
        var_name: &str,
        iterable: &TypedExpr,
        body: &TypedBlock,
    ) -> Result<(), CompileError> {
        self.begin_scope();

        self.compile_expr(iterable)?;
        let array_slot = self.declare_local("<for-array>");
        self.chunk.emit(OpCode::SetLocal(array_slot));
        self.chunk.emit(OpCode::Pop);

        let zero = self.const_int(0);
        self.chunk.emit(OpCode::Const(zero));
        let index_slot = self.declare_local("<for-index>");
        self.chunk.emit(OpCode::SetLocal(index_slot));
        self.chunk.emit(OpCode::Pop);

        let loop_start = self.chunk.next_index();
        // condition: index < len(array)
        self.chunk.emit(OpCode::GetLocal(index_slot));
        self.chunk.emit(OpCode::GetLocal(array_slot));
        self.chunk.emit(OpCode::ArrayLen);
        self.chunk.emit(OpCode::LtInt);
        let jump_if_false = self.chunk.emit(OpCode::JumpIfFalse(0));

        self.begin_scope();
        self.chunk.emit(OpCode::GetLocal(array_slot));
        self.chunk.emit(OpCode::GetLocal(index_slot));
        self.chunk.emit(OpCode::IndexGet);
        let elem_slot = self.declare_local(var_name);
        self.chunk.emit(OpCode::SetLocal(elem_slot));
        self.chunk.emit(OpCode::Pop);
        self.loop_stack.push(LoopContext { break_jumps: Vec::new(), continue_jumps: Vec::new(), handlers_at_entry: self.open_handlers });
        self.compile_block(body)?;
        let loop_ctx = self.loop_stack.pop().expect("pushed right above");
        self.end_scope();

        // continue targets HERE — the index increment — not `loop_start`.
        // Jumping straight to `loop_start` (the condition check) would
        // skip this increment entirely and spin forever re-checking the
        // same index.
        let increment_start = self.chunk.next_index();
        for continue_idx in &loop_ctx.continue_jumps {
            self.chunk.patch_jump(*continue_idx, increment_start);
        }

        // index = index + 1
        self.chunk.emit(OpCode::GetLocal(index_slot));
        let one = self.const_int(1);
        self.chunk.emit(OpCode::Const(one));
        self.chunk.emit(OpCode::AddInt);
        self.chunk.emit(OpCode::SetLocal(index_slot));
        self.chunk.emit(OpCode::Pop);

        self.chunk.emit(OpCode::Jump(loop_start));
        let after = self.chunk.next_index();
        self.chunk.patch_jump(jump_if_false, after);
        for break_idx in &loop_ctx.break_jumps {
            self.chunk.patch_jump(*break_idx, after);
        }

        self.end_scope();
        Ok(())
    }

    /// `for i in start..end [step s]`: a counting loop, no array. Shape:
    ///
    /// ```text
    ///   counter = start; stop = end; [step = check(s)]     (once, in source order)
    /// top:
    ///   if !(counter < stop) goto exit       (> stop for a negative step)
    ///   i = counter                          (a COPY: assigning `i` in the body
    ///                                         cannot change the iteration)
    ///   body
    /// next:                                  (`continue` lands here)
    ///   counter += step
    ///   goto top
    /// exit:                                  (`break` lands here)
    /// ```
    ///
    /// A missing or literal step has its direction fixed at compile time, so
    /// the hot loop is the same `compare + JumpIfFalse` shape as a `while`
    /// (the peephole fuses it). Only a non-literal step pays for a run-time
    /// sign test.
    fn compile_for_range(
        &mut self,
        var_name: &str,
        start: &TypedExpr,
        end: &TypedExpr,
        step: &RangeStep,
        var_assigned: bool,
        body: &TypedBlock,
    ) -> Result<(), CompileError> {
        self.begin_scope();

        self.compile_expr(start)?;
        let counter = self.declare_local("<range-counter>");
        self.chunk.emit(OpCode::SetLocal(counter));
        self.chunk.emit(OpCode::Pop);

        self.compile_expr(end)?;
        let stop = self.declare_local("<range-end>");
        self.chunk.emit(OpCode::SetLocal(stop));
        self.chunk.emit(OpCode::Pop);

        let step_slot = if let RangeStep::Dynamic(expr) = step {
            self.compile_expr(expr)?;
            self.chunk.emit(OpCode::CallBuiltin { id: spg_typecheck::BuiltinId::RangeStepCheck, argc: 1 });
            let slot = self.declare_local("<range-step>");
            self.chunk.emit(OpCode::SetLocal(slot));
            self.chunk.emit(OpCode::Pop);
            Some(slot)
        } else {
            None
        };

        // Nobody assigns to `i` in the body: the counter IS the variable (no
        // per-iteration copy; 2 instructions less in a 3-instruction loop).
        // The name is bound only now, after start/end/step were compiled, so
        // they still see any OUTER variable called `i`.
        if !var_assigned {
            self.scopes
                .last_mut()
                .expect("at least one scope active")
                .insert(var_name.to_string(), counter);
        }

        let loop_start = self.chunk.next_index();
        let mut exit_jumps: Vec<usize> = Vec::new();

        match step {
            RangeStep::One => self.emit_range_test(counter, stop, OpCode::LtInt, &mut exit_jumps),
            RangeStep::Const(n) if *n > 0 => {
                self.emit_range_test(counter, stop, OpCode::LtInt, &mut exit_jumps)
            }
            RangeStep::Const(_) => self.emit_range_test(counter, stop, OpCode::GtInt, &mut exit_jumps),
            RangeStep::Dynamic(_) => {
                let step_slot = step_slot.expect("dynamic step has a slot");
                // step > 0 ?
                self.chunk.emit(OpCode::GetLocal(step_slot));
                let zero = self.const_int(0);
                self.chunk.emit(OpCode::Const(zero));
                self.chunk.emit(OpCode::GtInt);
                let to_negative = self.chunk.emit(OpCode::JumpIfFalse(0));
                // ascending: counter < stop
                self.emit_range_test(counter, stop, OpCode::LtInt, &mut exit_jumps);
                let to_body = self.chunk.emit(OpCode::Jump(0));
                // descending: counter > stop
                let negative_start = self.chunk.next_index();
                self.chunk.patch_jump(to_negative, negative_start);
                self.emit_range_test(counter, stop, OpCode::GtInt, &mut exit_jumps);
                let body_start = self.chunk.next_index();
                self.chunk.patch_jump(to_body, body_start);
            }
        }

        self.begin_scope();
        if var_assigned {
            // the body reassigns `i`: it gets a COPY, so that cannot move the counter
            self.chunk.emit(OpCode::GetLocal(counter));
            let var_slot = self.declare_local(var_name);
            self.chunk.emit(OpCode::SetLocal(var_slot));
            self.chunk.emit(OpCode::Pop);
        }
        self.loop_stack.push(LoopContext { break_jumps: Vec::new(), continue_jumps: Vec::new(), handlers_at_entry: self.open_handlers });
        self.compile_block(body)?;
        let loop_ctx = self.loop_stack.pop().expect("pushed right above");
        self.end_scope();

        // `continue` must run the increment, never jump straight back to the test
        let next = self.chunk.next_index();
        for continue_idx in &loop_ctx.continue_jumps {
            self.chunk.patch_jump(*continue_idx, next);
        }

        self.chunk.emit(OpCode::GetLocal(counter));
        match step {
            RangeStep::One => {
                let one = self.const_int(1);
                self.chunk.emit(OpCode::Const(one));
            }
            RangeStep::Const(n) => {
                let k = self.const_int(*n);
                self.chunk.emit(OpCode::Const(k));
            }
            RangeStep::Dynamic(_) => {
                self.chunk.emit(OpCode::GetLocal(step_slot.expect("dynamic step has a slot")));
            }
        }
        self.chunk.emit(OpCode::AddInt);
        self.chunk.emit(OpCode::SetLocal(counter));
        self.chunk.emit(OpCode::Pop);

        self.chunk.emit(OpCode::Jump(loop_start));
        let after = self.chunk.next_index();
        for j in &exit_jumps {
            self.chunk.patch_jump(*j, after);
        }
        for break_idx in &loop_ctx.break_jumps {
            self.chunk.patch_jump(*break_idx, after);
        }

        self.end_scope();
        Ok(())
    }

    /// `counter <cmp> stop`, then a `JumpIfFalse` (recorded in `exits`, to be
    /// patched to the loop exit)
    fn emit_range_test(&mut self, counter: usize, stop: usize, cmp: OpCode, exits: &mut Vec<usize>) {
        self.chunk.emit(OpCode::GetLocal(counter));
        self.chunk.emit(OpCode::GetLocal(stop));
        self.chunk.emit(cmp);
        exits.push(self.chunk.emit(OpCode::JumpIfFalse(0)));
    }

    fn const_int(&mut self, n: i64) -> usize {
        self.chunk.add_constant(Value::Int(n))
    }

    // Expressions

    fn compile_expr(&mut self, expr: &TypedExpr) -> Result<(), CompileError> {
        match expr {
            TypedExpr::IntLit(n) => {
                let idx = self.chunk.add_constant(Value::Int(*n));
                self.chunk.emit(OpCode::Const(idx));
            }
            TypedExpr::FloatLit(n) => {
                let idx = self.chunk.add_constant(Value::Float(*n));
                self.chunk.emit(OpCode::Const(idx));
            }
            TypedExpr::StringLit(s) => {
                let idx = self.chunk.add_constant(Value::String(std::rc::Rc::new(s.clone())));
                self.chunk.emit(OpCode::Const(idx));
            }
            TypedExpr::BoolLit(b) => {
                let idx = self.chunk.add_constant(Value::Bool(*b));
                self.chunk.emit(OpCode::Const(idx));
            }
            TypedExpr::ArrayLit { items, .. } => {
                for item in items {
                    self.compile_expr(item)?;
                }
                self.chunk.emit(OpCode::MakeArray(items.len()));
            }

            TypedExpr::Ident { name, .. } => match self.resolve_var(name) {
                Some(VarSlot::Local(slot)) => {
                    self.chunk.emit(OpCode::GetLocal(slot));
                }
                Some(VarSlot::Global(slot)) => {
                    self.chunk.emit(OpCode::GetGlobal(slot));
                }
                // not a variable: a named function used as a value (no captures)
                None => {
                    let func_index = *self.function_index.get(name).ok_or_else(|| CompileError::UnknownFunction(name.clone()))?;
                    self.chunk.emit(OpCode::MakeClosure { func_index, ncap: 0 });
                }
            },

            TypedExpr::NoneLit { .. } => {
                let idx = self.chunk.add_constant(Value::Nil);
                self.chunk.emit(OpCode::Const(idx));
            }

            // `left ?? right`: the default is only computed when `left` is none
            TypedExpr::Coalesce { left, right, .. } => {
                self.compile_expr(left)?;
                let keep = self.chunk.emit(OpCode::JumpIfNotNone(0));
                self.compile_expr(right)?;
                let after = self.chunk.next_index();
                self.chunk.patch_jump(keep, after);
            }

            TypedExpr::Lambda { params, body, captures, return_type, .. } => {
                self.compile_lambda(params, body, captures, return_type)?;
            }

            TypedExpr::CallValue { callee, args, .. } => {
                self.compile_expr(callee)?;
                for arg in args {
                    self.compile_expr(arg)?;
                }
                self.chunk.emit(OpCode::CallValue { argc: args.len() });
            }

            TypedExpr::Binary { left, op, right, .. } => {
                self.compile_binary(left, op, right)?;
            }

            TypedExpr::Unary { op, expr, .. } => {
                self.compile_expr(expr)?;
                let opcode = match (op, expr.ty()) {
                    (UnaryOp::Neg, Ty::Int) => OpCode::NegInt,
                    (UnaryOp::Neg, Ty::Float) => OpCode::NegFloat,
                    (UnaryOp::Not, Ty::Bool) => OpCode::Not,
                    (op, ty) => panic!("compiler bug: unary {:?} on {:?} passed typecheck but has no opcode", op, ty),
                };
                self.chunk.emit(opcode);
            }

            TypedExpr::Call { callee, args, builtin, .. } => {
                self.compile_call(callee, args, *builtin)?;
            }

            TypedExpr::Index { array, index, .. } => {
                self.compile_expr(array)?;
                self.compile_expr(index)?;
                self.chunk.emit(if matches!(array.ty(), Ty::Map(..)) { OpCode::MapGet } else { OpCode::IndexGet });
            }

            TypedExpr::MapLit { entries, .. } => {
                for (k, v) in entries {
                    self.compile_expr(k)?;
                    self.compile_expr(v)?;
                }
                self.chunk.emit(OpCode::MakeMap(entries.len()));
            }

            TypedExpr::StructLit { name, fields, .. } => {
                // push order matches OpCode::MakeStruct's expected pop
                // order — `fields` is already in declaration order
                // (spg-typecheck's Expr::StructLit handling reorders it),
                // matching struct_defs' field_names order 1:1
                for f in fields {
                    self.compile_expr(f)?;
                }
                let def_index = *self.struct_table.name_to_index.get(name).ok_or_else(|| {
                    CompileError::InvalidControlFlow(format!(
                        "compiler bug: struct literal for unregistered struct '{}' (should've been caught by typecheck)",
                        name
                    ))
                })?;
                self.chunk.emit(OpCode::MakeStruct { def_index, field_count: fields.len() });
            }

            TypedExpr::EnumLit { name, index, .. } => {
                let def = *self.struct_table.enum_indices.get(name).ok_or_else(|| CompileError::UnknownEnum(name.clone()))?;
                let idx = self.chunk.add_constant(Value::Enum(def, *index as u32));
                self.chunk.emit(OpCode::Const(idx));
            }
            TypedExpr::EnumValue { operand, .. } => {
                // `Color.Red.value` is known at compile time: a plain constant
                if let TypedExpr::EnumLit { name, index, .. } = operand.as_ref() {
                    let def = *self.struct_table.enum_indices.get(name).ok_or_else(|| CompileError::UnknownEnum(name.clone()))?;
                    let value = self.struct_table.enum_values[def as usize][*index].clone();
                    let idx = self.chunk.add_constant(value);
                    self.chunk.emit(OpCode::Const(idx));
                } else {
                    self.compile_expr(operand)?;
                    self.chunk.emit(OpCode::EnumValue);
                }
            }
            // reads an enum as the int the compare opcodes expect: no code of its own
            TypedExpr::AsInt(inner) => self.compile_expr(inner)?,
            TypedExpr::Field { object, field, .. } => {
                self.compile_expr(object)?;
                let field_index = self.resolve_field_index(object.ty(), field)?;
                self.chunk.emit(OpCode::FieldGet { field_index });
            }

            TypedExpr::IfExpr { cond, then_branch, else_branch, .. } => {
                self.compile_expr(cond)?;
                let jump_if_false = self.chunk.emit(OpCode::JumpIfFalse(0));
                self.compile_block(then_branch)?;
                let jump_over_else = self.chunk.emit(OpCode::Jump(0));
                let else_start = self.chunk.next_index();
                self.chunk.patch_jump(jump_if_false, else_start);
                self.compile_block(else_branch)?;
                let after = self.chunk.next_index();
                self.chunk.patch_jump(jump_over_else, after);
            }
        }
        Ok(())
    }

    fn compile_binary(&mut self, left: &TypedExpr, op: &BinaryOp, right: &TypedExpr) -> Result<(), CompileError> {
        // And/Or short-circuit: the right side is only evaluated when
        // it can still change the result, so these compile to jumps
        // instead of an eager two-value opcode.
        if matches!(op, BinaryOp::And) {
            self.compile_expr(left)?;
            // if left is false, short-circuit: jump straight past the
            // right-hand evaluation, leaving `false` as the result
            let jump_if_false = self.chunk.emit(OpCode::JumpIfFalse(0));
            self.compile_expr(right)?;
            let jump_over_false = self.chunk.emit(OpCode::Jump(0));
            let push_false_at = self.chunk.next_index();
            self.chunk.patch_jump(jump_if_false, push_false_at);
            let idx = self.chunk.add_constant(Value::Bool(false));
            self.chunk.emit(OpCode::Const(idx));
            let after = self.chunk.next_index();
            self.chunk.patch_jump(jump_over_false, after);
            return Ok(());
        }
        if matches!(op, BinaryOp::Or) {
            self.compile_expr(left)?;
            let jump_if_false = self.chunk.emit(OpCode::JumpIfFalse(0));
            let idx = self.chunk.add_constant(Value::Bool(true));
            self.chunk.emit(OpCode::Const(idx));
            let jump_over_right = self.chunk.emit(OpCode::Jump(0));
            let eval_right_at = self.chunk.next_index();
            self.chunk.patch_jump(jump_if_false, eval_right_at);
            self.compile_expr(right)?;
            let after = self.chunk.next_index();
            self.chunk.patch_jump(jump_over_right, after);
            return Ok(());
        }

        self.compile_expr(left)?;
        self.compile_expr(right)?;
        // an optional on either side (`x == none`, `x == 3` with `x: int?`) compares
        // whole values: the int/float/string opcodes would reject a `none`
        let optional = |t: &Ty| matches!(t, Ty::Optional(_) | Ty::NoneLit);
        if matches!(op, BinaryOp::Eq | BinaryOp::NotEq) && (optional(left.ty()) || optional(right.ty())) {
            self.chunk.emit(if matches!(op, BinaryOp::Eq) { OpCode::EqValue } else { OpCode::NotEqValue });
            return Ok(());
        }
        let left_ty = left.ty();
        let opcode = binary_opcode(op, left_ty);
        self.chunk.emit(opcode);
        Ok(())
    }

    /// a lambda: its body becomes a chunk in the shared sink (reserved first, so
    /// its index is known before its body is compiled); here, the captured values
    /// are pushed in the order the body expects them, then the closure is made
    fn compile_lambda(
        &mut self,
        params: &[spg_parser::ast::Param],
        body: &TypedBlock,
        captures: &[(String, Ty)],
        return_type: &Ty,
    ) -> Result<(), CompileError> {
        let (base, index) = {
            let mut sink = self.sink.borrow_mut();
            let index = sink.base + sink.chunks.len();
            sink.chunks.push(Chunk::new("<lambda>"));
            (sink.base, index)
        };

        for (name, _) in captures {
            match self.resolve_var(name) {
                Some(VarSlot::Local(slot)) => self.chunk.emit(OpCode::GetLocal(slot)),
                Some(VarSlot::Global(slot)) => self.chunk.emit(OpCode::GetGlobal(slot)),
                None => return Err(CompileError::UnknownFunction(name.clone())),
            };
        }

        let mut fc = FunctionCompiler::new("<lambda>", self.function_index, self.struct_table, self.globals, false);
        fc.sink = self.sink.clone();
        fc.chunk.cur_loc = self.chunk.cur_loc.clone();
        fc.chunk.arity = params.len();
        fc.begin_scope();
        // captured values arrive first, then the arguments (see OpCode::CallValue)
        for (name, _) in captures {
            fc.declare_local(name);
        }
        for p in params {
            fc.declare_local(&p.name);
        }
        fc.compile_block(body)?;
        fc.emit_implicit_return(return_type);
        fc.end_scope();

        self.sink.borrow_mut().chunks[index - base] = fc.chunk;
        self.chunk.emit(OpCode::MakeClosure { func_index: index, ncap: captures.len() });
        Ok(())
    }

    fn compile_call(
        &mut self,
        callee: &TypedExpr,
        args: &[TypedExpr],
        builtin: Option<spg_typecheck::BuiltinId>,
    ) -> Result<(), CompileError> {
        let name = match callee {
            TypedExpr::Ident { name, .. } => name.clone(),
            _ => return Err(CompileError::InvalidControlFlow("call to non-identifier callee".to_string())),
        };

        for arg in args {
            self.compile_expr(arg)?;
        }

        // the checker already resolved which builtin (if any) this call
        // targets and stashed it on the typed node — the compiler just
        // reads that instead of re-matching the name itself, which is
        // what lets `compile_expr`'s `TypedExpr::Call { .. }` arm stay a
        // single call to this function regardless of overloads
        if let Some(id) = builtin {
            self.chunk.emit(OpCode::CallBuiltin { id, argc: args.len() });
            return Ok(());
        }

        let func_index =
            *self.function_index.get(&name).ok_or_else(|| CompileError::UnknownFunction(name.clone()))?;
        self.chunk.emit(OpCode::Call { func_index, argc: args.len() });
        Ok(())
    }
}

fn binary_opcode(op: &BinaryOp, left_ty: &Ty) -> OpCode {
    use BinaryOp::*;
    use Ty::*;
    match (op, left_ty) {
        (Add, Int) => OpCode::AddInt,
        (Sub, Int) => OpCode::SubInt,
        (Mul, Int) => OpCode::MulInt,
        (Div, Int) => OpCode::DivInt,
        (Mod, Int) => OpCode::ModInt,
        (Pow, Int) => OpCode::PowInt,
        (Add, Float) => OpCode::AddFloat,
        (Sub, Float) => OpCode::SubFloat,
        (Mul, Float) => OpCode::MulFloat,
        (Div, Float) => OpCode::DivFloat,
        (Mod, Float) => OpCode::ModFloat,
        (Pow, Float) => OpCode::PowFloat,
        (Add, String) => OpCode::ConcatString,

        (Eq, Int) => OpCode::EqInt,
        (Eq, Float) => OpCode::EqFloat,
        (Eq, Bool) => OpCode::EqBool,
        (Eq, String) => OpCode::EqString,
        (NotEq, Int) => OpCode::NotEqInt,
        (NotEq, Float) => OpCode::NotEqFloat,
        (NotEq, Bool) => OpCode::NotEqBool,
        (NotEq, String) => OpCode::NotEqString,

        (Lt, Int) => OpCode::LtInt,
        (Gt, Int) => OpCode::GtInt,
        (LtEq, Int) => OpCode::LtEqInt,
        (GtEq, Int) => OpCode::GtEqInt,
        (Lt, Float) => OpCode::LtFloat,
        (Gt, Float) => OpCode::GtFloat,
        (LtEq, Float) => OpCode::LtEqFloat,
        (GtEq, Float) => OpCode::GtEqFloat,

        // arrays, maps and structs compare by CONTENT (deep), not by identity
        (Eq, Array(_) | Map(..) | Named(_)) => OpCode::EqValue,
        (NotEq, Array(_) | Map(..) | Named(_)) => OpCode::NotEqValue,

        (op, ty) => panic!("compiler bug: binary {:?} on {:?} passed typecheck but has no opcode", op, ty),
    }
}

/// pulls the catch variable name back out of a `TypedStmt::Try` — small
/// helper so `compile_stmt`'s match arm doesn't need to destructure the
/// field twice (once for the match guard, once to use the value)
fn catch_var_name(stmt: &TypedStmt) -> String {
    match stmt {
        TypedStmt::Try { catch_var, .. } => catch_var.clone(),
        _ => unreachable!("catch_var_name called on non-Try statement"),
    }
}

/// typed-AST location -> bytecode location (the file name is shared, not copied per op)
fn to_loc(loc: &spg_typecheck::SrcLoc) -> spg_vm::chunk::Loc {
    spg_vm::chunk::Loc {
        file: loc.file.as_deref().map(std::rc::Rc::from),
        line: loc.line as u32,
    }
}
