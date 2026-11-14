//! the typechecker itself. walks the parser's AST, resolves scope with a
//! stack of symbol tables, checks types, and either spits out a fully
//! typed AST or collects every TypeError it finds on the way. thorough.
//!
//! scope model: one HashMap<String, Ty> per active block, pushed and popped
//! as blocks get entered and left (function bodies, if/while/for/try bodies).
//! a variable is visible in its own block and any nested block. same as
//! C/Rust/JS. function and struct names live in
//! one separate global table (no nested function declarations in
//! v0.1, keeps the resolver sane).

use std::collections::HashMap;

use spg_parser::ast::{self as p, BinaryOp, Expr, Param, Stmt, TypeExpr, UnaryOp};

use crate::typed_ast::{ArrayFnKind, MapFnKind};
use crate::builtins::{
    builtin_signatures, method_namespace, native_array_method_names, native_method_names, BuiltinId,
};
use crate::error::TypeError;
use crate::typed_ast::{RangeStep, SrcLoc, Ty, TypedBlock, TypedExpr, TypedProgram, TypedStmt};

#[derive(Clone)]
struct FunctionSig {
    params: Vec<Param>,
    return_type: Ty,
}

/// a lambda being checked: where its own scopes start, and the outer locals it grabs
struct LambdaFrame {
    base: usize,
    captures: Vec<(String, Ty)>,
}

/// what the scope stack tracks per name: type, plus whether
/// reassignment is allowed, so `const` gets enforced at the point of
/// `x = ...`, not only at the declaration
#[derive(Clone)]
struct VarInfo {
    ty: Ty,
    mutable: bool,
    /// set once a successfully-checked `name = ...` resolves to THIS binding
    /// (shadowing-exact, because it's the binding's own flag). a `for i in
    /// a..b` reads it for its own variable, so the compiler can skip a copy
    /// when nobody touches `i`. lazy and smart.
    assigned: bool,
    /// set on a narrowed entry (`x != none` proved an `int?` is an `int`): the
    /// scope index of the real binding, so captures and assignments still find it
    origin: Option<usize>,
    /// the type the variable has outside the narrowing. an assignment ends it.
    narrowed_from: Option<Ty>,
}

/// one declaration of a value-type method (`fn string.nice`). `internal` is the name the
/// function is compiled under: `string.nice` when exported, module-tagged otherwise
#[derive(Debug, Clone)]
struct ValueMethod {
    internal: String,
    exported: bool,
    file: Option<String>,
    /// registration order across all value types: `int::methods` lists user methods by it
    seq: usize,
}

pub struct Checker {
    functions: HashMap<String, FunctionSig>,
    /// `"string.nice"` -> every declaration of that method in the program
    value_methods: HashMap<String, Vec<ValueMethod>>,
    /// struct name -> its fields in declaration order (order matters for
    /// error messages, and maybe a future positional-literal
    /// syntax (even though today's `Name { field: value }` is
    /// checked by name, not position). future problems, future us.
    struct_defs: HashMap<String, Vec<Param>>,
    /// structs with `guard` blocks: every write to them is wrapped (see `wrap_guarded_write`)
    guarded_structs: std::collections::HashSet<String>,
    /// positions of struct literals already wrapped in `Name.__check(...)`, so the
    /// literal inside the call is checked as itself and not wrapped again
    guard_literals: std::collections::HashSet<(usize, usize)>,
    /// enum name -> its variants in declaration order (a variant's position
    /// is its identity at runtime). an enum value's type is `Ty::Named(name)`;
    /// this table is how it's told apart from a struct.
    enum_defs: HashMap<String, Vec<String>>,
    /// enum name -> the type of its variants' values (`c.value`)
    enum_value_tys: HashMap<String, Ty>,
    scopes: Vec<HashMap<String, VarInfo>>,
    /// inside `fn som.x` bodies: the namespace name and the scope depth where its function starts
    namespace_ctx: Option<(String, usize)>,
    /// names of `namespace` declarations of this program
    namespace_names: std::collections::HashSet<String>,
    /// `ns.member` (function or static) -> (exported, file that declares it)
    member_visibility: HashMap<String, (bool, Option<String>)>,
    /// `#> strict` (set for the whole program. only the entry file has directives)
    strict: bool,
    errors: Vec<TypeError>,
    /// how many while/for-in loops we're nested inside right now.
    /// used to reject break/continue outside a loop. saved and restored
    /// around function bodies, so a loop outside can never
    /// "leak" into a function declared inside it (moot today, since
    /// v1.0 has no nested functions, but this keeps the check
    /// correct if that ever changes. future-proofing, baby.
    loop_depth: usize,
    /// names of functions whose return type inference (collect_function_
    /// signatures, pass B) failed because their own returns disagreed.
    /// check_all_paths_return skips its normal return-type comparison
    /// for these. the real problem was already reported as
    /// CannotInferReturnType, and comparing bodies to the Void
    /// placeholder infer_return_type falls back to would just make
    /// confusing, redundant ReturnTypeMismatch noise on top. nobody needs that.
    uninferrable_functions: std::collections::HashSet<String>,
    /// `Type.name` functions declared at top level with no `me`: associated functions
    associated: std::collections::HashSet<String>,
    /// enum matches with no `else` that name every variant, keyed by their
    /// position. they always end in one of their arms, so the return-path
    /// check can count them as returning. trust the enum.
    exhaustive_matches: std::collections::HashSet<(Option<String>, usize, usize)>,
    /// declared/inferred return type of the function whose body is being
    /// checked RIGHT NOW (`None` at the root, outside any function).
    /// `Stmt::Return` compares against this in the same pass that checks
    /// the returned expression. a second walk just to compare types
    /// would re-run `check_expr` and re-print every error inside. no thanks.
    current_return: Option<Ty>,
    /// lambdas whose bodies are being checked, outermost first
    lambdas: Vec<LambdaFrame>,
    /// the native objects of the libraries this program installs: struct name -> (module, object).
    /// their fields and methods are lowered to calls of the module's functions (see `lower`)
    native_objects: HashMap<String, (&'static str, &'static spg_libs::NativeObject)>,
}

/// the native objects of the installed libraries, by struct name
fn native_objects_of(natives: &[String]) -> HashMap<String, (&'static str, &'static spg_libs::NativeObject)> {
    let mut out = HashMap::new();
    for name in natives {
        let Some(module) = spg_libs::module(name) else { continue };
        for object in module.objects {
            out.insert(object.name.to_string(), (module.name, object));
        }
    }
    out
}

pub fn type_check(program: p::Program) -> Result<TypedProgram, Vec<TypeError>> {
    type_check_with(program, false)
}

/// `strict` is the file's `#> strict`. it only HARDENS the rules (hidden names and
/// dead code become errors). the language accepted is the same
pub fn type_check_with(program: p::Program, strict: bool) -> Result<TypedProgram, Vec<TypeError>> {
    let native_objects = native_objects_of(&program.natives);
    let mut checker = Checker {
        native_objects,
        functions: HashMap::new(),
        value_methods: HashMap::new(),
        struct_defs: HashMap::new(),
        guarded_structs: std::collections::HashSet::new(),
        guard_literals: std::collections::HashSet::new(),
        enum_defs: HashMap::new(),
        enum_value_tys: HashMap::new(),
        scopes: vec![HashMap::new()],
        namespace_ctx: None,
        namespace_names: std::collections::HashSet::new(),
        member_visibility: HashMap::new(),
        errors: Vec::new(),
        loop_depth: 0,
        uninferrable_functions: std::collections::HashSet::new(),
        associated: std::collections::HashSet::new(),
        exhaustive_matches: std::collections::HashSet::new(),
        current_return: None,
        lambdas: Vec::new(),
        strict,
    };

    checker.collect_struct_signatures(&program.stmts);
    checker.collect_function_signatures(&program.stmts);

    let mut typed_stmts = Vec::new();
    let mut locs = Vec::new();
    let mut last_loc = SrcLoc::default();
    for stmt in &program.stmts {
        let loc = stmt_loc(stmt, &last_loc);
        last_loc = loc.clone();
        if let Some(typed) = checker.check_stmt(stmt) {
            typed_stmts.push(typed);
            locs.push(loc);
        }
    }

    // a guard must not reach a write to its own struct (only when the rest is fine)
    if checker.errors.is_empty() {
        let mut positions = HashMap::new();
        for stmt in &program.stmts {
            if let p::Stmt::FunctionDecl { name, pos, .. } = stmt {
                if name.ends_with(".__guard") {
                    positions.insert(name.clone(), pos.clone());
                }
            }
        }
        let reentry = crate::guard_reentry::check(&typed_stmts, &positions);
        checker.errors.extend(reentry);
    }

    if checker.errors.is_empty() {
        Ok(TypedProgram { stmts: typed_stmts, locs })
    } else {
        Err(checker.errors)
    }
}

impl Checker {
    // setup: register every function's signature before checking
    // bodies, so forward calls (fn A calls fn B declared below it)
    // resolve correctly. the way it should be.

    fn collect_struct_signatures(&mut self, stmts: &[Stmt]) {
        // structs have no inference to do (every field is always
        // explicitly typed, like a function param). one pass is
        // enough, unlike collect_function_signatures's two-pass dance
        // for return-type inference. order among structs doesn't matter
        // for the same reason `array<T>` doesn't care
        // about declaration order: field types are just *names* here
        // (Ty::Named) until something actually builds or reads a
        // field, and by then every struct is already registered.
        for stmt in stmts {
            match stmt {
                // (a struct and an enum with the same name never get here. the
                // module resolver already yells about duplicate names)
                Stmt::StructDecl { name, fields, guarded, namespace, .. } => {
                    self.struct_defs.insert(name.clone(), fields.clone());
                    if *namespace {
                        self.namespace_names.insert(name.clone());
                    }
                    if *guarded {
                        self.guarded_structs.insert(name.clone());
                    }
                }
                Stmt::EnumDecl { name, variants, values, .. } => {
                    self.enum_defs.insert(name.clone(), variants.clone());
                    let value_ty = match values.first() {
                        Some(spg_parser::ast::EnumValue::Float(_)) => Ty::Float,
                        Some(spg_parser::ast::EnumValue::Str(_)) => Ty::String,
                        Some(spg_parser::ast::EnumValue::Bool(_)) => Ty::Bool,
                        _ => Ty::Int,
                    };
                    self.enum_value_tys.insert(name.clone(), value_ty);
                }
                _ => {}
            }
        }
    }

    /// true when `ty` is the type of an enum's values
    fn is_enum_ty(&self, ty: &Ty) -> bool {
        matches!(ty, Ty::Named(n) if self.enum_defs.contains_key(n))
    }

    /// `name` read as an enum. only when no variable/param of that name
    /// is in scope (a local with the enum's name shadows it, like any name)
    fn enum_named(&self, name: &str) -> Option<&Vec<String>> {
        if self.lookup(name).is_some() {
            return None;
        }
        self.enum_defs.get(name)
    }

    fn collect_function_signatures(&mut self, stmts: &[Stmt]) {
        // associated functions: `function Type.name(...)` with no `me`. the type must be a
        // struct of this program; the names are remembered so calls can be told apart
        for stmt in stmts {
            // a namespace's member (function or static) remembers whether it is exported, and where
            match stmt {
                Stmt::FunctionDecl { name, exported, pos, .. } if name.contains('.') => {
                    self.member_visibility.insert(name.clone(), (*exported, pos.file.clone()));
                }
                Stmt::Let { name, exported, pos, .. } | Stmt::Const { name, exported, pos, .. } if name.contains('.') => {
                    self.member_visibility.insert(name.clone(), (*exported, pos.file.clone()));
                }
                _ => {}
            }
            if let Stmt::FunctionDecl { name, params, pos, exported, .. } = stmt {
                // a module-tagged name (`main#9e60::string.nice`) is the same method as `string.nice`
                let shown = spg_parser::ast::display_name(name);
                if let Some((type_name, member)) = shown.split_once('.') {
                    // the struct is registered under its MANGLED name (`arith#e5fe::arith`, after an
                    // install), so the owner is the full name minus `.member`. the bare `type_name`
                    // (`arith`) only exists in the entry file, where the two are the same
                    let full_type = &name[..name.len() - member.len() - 1];
                    // `string.name`, desugared from `string::methods.add(fn name(...))`: `me` is always its
                    // first parameter, so it is a plain function, not an associated one
                    if let Some(value_ty) = value_type_of(type_name) {
                        if member == "string" || member == "int" || member == "float" || member == "copy"
                            || method_namespace(&value_ty, member).is_some()
                        {
                            self.errors.push(TypeError::ValueMethodClash {
                                type_name: type_name.to_string(),
                                name: member.to_string(),
                                pos: pos.clone(),
                            });
                        } else {
                            // two declarations clash when one of them is exported (seen everywhere)
                            // or both live in the same file. a private method of another module is no clash
                            let key = format!("{}.{}", type_name, member);
                            let clash = self.value_methods.get(&key).map_or(false, |es| {
                                es.iter().any(|e| e.exported || *exported || e.file == pos.file)
                            });
                            if clash {
                                self.errors.push(TypeError::ValueMethodDuplicate {
                                    type_name: type_name.to_string(),
                                    name: member.to_string(),
                                    pos: pos.clone(),
                                });
                            }
                            let seq = self.value_methods.values().map(|v| v.len()).sum();
                            self.value_methods.entry(key).or_default().push(ValueMethod {
                                internal: name.clone(),
                                exported: *exported,
                                file: pos.file.clone(),
                                seq,
                            });
                        }
                        continue;
                    }
                    let is_method = params.first().map_or(false, |p| p.name == "me");
                    if !is_method {
                        if !self.is_type_name(full_type) {
                            self.errors.push(TypeError::AssociatedOnUnknownType {
                                type_name: type_name.to_string(),
                                name: name.clone(),
                                pos: pos.clone(),
                            });
                        } else if self.enum_defs.get(full_type).map_or(false, |vs| vs.iter().any(|v| v == member_of(name))) {
                            self.errors.push(TypeError::AssociatedNameIsVariant {
                                type_name: type_name.to_string(),
                                name: member_of(name).to_string(),
                                pos: pos.clone(),
                            });
                        }
                        self.associated.insert(name.clone());
                    }
                }
            }
        }

        // pass A: register every function that HAS an explicit `-> Type`
        // first, so inference for the un-annotated ones (pass B) can see
        // every explicitly typed function, not just the ones declared
        // earlier in the file. matches the "forward calls work" promise.
        // explicit signatures already had. consistency, friend.
        for stmt in stmts {
            if let Stmt::FunctionDecl { name, params, return_type: Some(rt), .. } = stmt {
                self.functions.insert(
                    name.clone(),
                    FunctionSig { params: params.clone(), return_type: resolve_type_expr(rt) },
                );
            }
        }

        // pass B: infer the rest by scanning each one's own returns.
        // order among un-annotated functions doesn't matter
        // UNLESS two of them call each other, and neither has a type.
        // that mutual case isn't supported. (most languages say the same.)
        // the fix is on you: annotate one of the two. sorry.
        // have); the fix from the caller's side is just to annotate one
        for stmt in stmts {
            if let Stmt::FunctionDecl { name, params, return_type: None, body, pos, .. } = stmt {
                let inferred = self.infer_return_type(name, params, body, pos);
                self.functions.insert(name.clone(), FunctionSig { params: params.clone(), return_type: inferred });
            }
        }
    }

    /// figures out a function's return type by scanning its body for
    /// `return` statements, WITHOUT emitting its own errors.
    /// it snapshots and restores `self.errors` around the scan, so any
    /// errors check_expr throws while inferring get discarded. the real
    /// check_stmt pass over the body (which runs after) is
    /// the one that actually shows real errors to the user. params ARE
    /// declared for real in this temp scope (not left undefined),
    /// since something like `return x + 1` needs `x`'s real type to
    /// infer anything useful. a placeholder type here could easily
    /// give a wrong return type, not just a wrong error. nasty.
    ///
    /// no `return` anywhere -> Void (like a function that only
    /// falls off the end). all `return`s agree -> that type.
    /// they disagree -> CannotInferReturnType. Void is the placeholder,
    /// so the rest of typechecking has *something* to chew on instead of
    /// panicking. the real error is what the user sees. this placeholder
    /// never reaches a successful Ok result. never. promise.
    fn infer_return_type(&mut self, name: &str, params: &[p::Param], body: &p::Block, pos: &p::Pos) -> Ty {
        // errors of this probe pass get thrown away. the real pass reports them.
        let saved_errors = std::mem::take(&mut self.errors);
        self.push_scope();
        for param in params {
            self.declare(&param.name, resolve_type_expr(&param.type_ann), true, pos);
        }

        let mut found: Vec<Ty> = Vec::new();
        self.collect_return_types_in_stmts(&body.stmts, &mut found);
        self.errors = saved_errors;

        self.pop_scope();

        if found.is_empty() {
            return Ty::Void;
        }
        let first = found[0].clone();
        if found.iter().all(|t| *t == first) {
            first
        } else {
            self.errors.push(TypeError::CannotInferReturnType {
                function_name: name.to_string(),
                found_types: found,
                pos: pos.clone(),
            });
            self.uninferrable_functions.insert(name.to_string());
            Ty::Void
        }
    }

    /// walks the body COLLECTING the type of every `return` it finds.
    /// only used during return-type inference, before there's an
    /// expected type to compare to. (the real comparison happens in
    /// `check_stmt`'s `Stmt::Return` arm, in the same pass that checks
    /// the returned expression.) efficient, unlike my life choices.
    fn collect_return_types_in_stmts(&mut self, stmts: &[Stmt], found: &mut Vec<Ty>) {
        for stmt in stmts {
            match stmt {
                Stmt::Return { value, .. } => {
                    let ty = match value {
                        Some(expr) => self.infer_expr_ty(expr),
                        None => Ty::Void,
                    };
                    found.push(ty);
                }
                Stmt::IfStmt { then_branch, else_branch, .. } => {
                    self.collect_return_types_in_stmts(&then_branch.stmts, found);
                    if let Some(eb) = else_branch {
                        self.collect_return_types_in_stmts(&eb.stmts, found);
                    }
                }
                Stmt::While { body, .. } => self.collect_return_types_in_stmts(&body.stmts, found),
                Stmt::CompileVm { body, .. } => self.collect_return_types_in_stmts(&body.stmts, found),
                Stmt::CompileJit { body, .. } => self.collect_return_types_in_stmts(&body.stmts, found),
                Stmt::DoWhile { body, .. } => self.collect_return_types_in_stmts(&body.stmts, found),
                Stmt::Match { arms, else_branch, .. } => {
                    for arm in arms {
                        self.collect_return_types_in_stmts(&arm.body.stmts, found);
                    }
                    if let Some(eb) = else_branch {
                        self.collect_return_types_in_stmts(&eb.stmts, found);
                    }
                }
                Stmt::ForIn { body, .. } => self.collect_return_types_in_stmts(&body.stmts, found),
                Stmt::ForRange { body, .. } => self.collect_return_types_in_stmts(&body.stmts, found),
                Stmt::Try { try_block, catch_block, .. } => {
                    self.collect_return_types_in_stmts(&try_block.stmts, found);
                    self.collect_return_types_in_stmts(&catch_block.stmts, found);
                }
                _ => {}
            }
        }
    }

    // scope helpers (the boring glue)

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn declare(&mut self, name: &str, ty: Ty, mutable: bool, pos: &p::Pos) {
        // `#> strict`: a name can't hide one from an outer scope (names that
        // start with `_`, compiler-made names and a method's implicit `me` are exempt)
        if self.strict && name != "me" && name.chars().next().is_some_and(|c| c.is_alphabetic()) {
            let outer = &self.scopes[..self.scopes.len() - 1];
            if outer.iter().any(|scope| scope.contains_key(name)) {
                self.errors.push(TypeError::ShadowedDeclaration { name: name.to_string(), pos: pos.clone() });
            }
        }
        let current = self.scopes.last_mut().expect("at least one scope always active");
        if current.contains_key(name) {
            self.errors.push(TypeError::DuplicateDeclaration {
                name: name.to_string(),
                pos: pos.clone(),
            });
            return;
        }
        current.insert(name.to_string(), VarInfo { ty, mutable, assigned: false, origin: None, narrowed_from: None });
    }

    fn lookup(&self, name: &str) -> Option<&Ty> {
        for scope in self.scopes.iter().rev() {
            if let Some(info) = scope.get(name) {
                return Some(&info.ty);
            }
        }
        None
    }

    /// a read of `name` as an expression. a local of an enclosing function, seen
    /// from inside a lambda, becomes a capture of every lambda it crosses (so the
    /// nested ones pass it along). globals (root scope) are never captured. they're global.
    fn resolve_name(&mut self, name: &str) -> Option<Ty> {
        let (scope, ty) = self
            .scopes
            .iter()
            .enumerate()
            .rev()
            .find_map(|(i, scope)| scope.get(name).map(|info| (info.origin.unwrap_or(i), info.ty.clone())))?;
        if scope >= 1 {
            for frame in self.lambdas.iter_mut() {
                if scope < frame.base && !frame.captures.iter().any(|(n, _)| n == name) {
                    frame.captures.push((name.to_string(), ty.clone()));
                }
            }
        }
        Some(ty)
    }

    /// true when `name` is a local of an enclosing function that the innermost
    /// lambda reaches across its border (assigning would only change a copy)
    fn assigns_capture(&self, name: &str) -> bool {
        let scope = self.scopes.iter().enumerate().rev().find_map(|(i, s)| s.get(name).map(|info| info.origin.unwrap_or(i)));
        match (scope, self.lambdas.last()) {
            (Some(scope), Some(frame)) => scope >= 1 && scope < frame.base,
            _ => false,
        }
    }

    /// same lookup, but returns the full VarInfo. used by Assign, which
    /// (unlike every other read site) also needs to know about mutability
    /// a struct or enum declared in this program: the types that take associated functions
    fn is_type_name(&self, name: &str) -> bool {
        self.struct_defs.contains_key(name) || self.enum_defs.contains_key(name)
    }

    /// inside a namespace function, a bare name that is not a local means the namespace's own
    /// static (`calls` -> `som.calls`). a local of the same name always wins.
    fn namespace_member(&self, name: &str) -> Option<String> {
        let (ns, depth) = self.namespace_ctx.as_ref()?;
        let local = self.scopes.iter().enumerate().rev().find(|(_, s)| s.contains_key(name)).map(|(i, _)| i);
        if local.map_or(false, |i| i >= *depth) {
            return None;
        }
        let full = format!("{}.{}", ns, name);
        (self.lookup_var(&full).is_some() || self.functions.contains_key(&full)).then_some(full)
    }

    /// `ns.field = ...` where `ns` is a namespace and `field` one of its statics
    fn namespace_write_target(&self, object: &Expr, field: &str) -> Option<String> {
        if let Expr::Ident(ns, _) = object {
            if self.lookup_var(ns).is_none() && self.lookup_var(&format!("{}.{}", ns, field)).is_some() {
                return Some(ns.clone());
            }
        }
        None
    }

    /// a namespace member used from another file must be exported. `Some((namespace, member))` when it is not
    fn namespace_private(&self, full: &str, pos: &p::Pos) -> Option<(String, String)> {
        let (ns, member) = full.rsplit_once('.')?;
        if !self.namespace_names.contains(ns) {
            return None;
        }
        match self.member_visibility.get(full) {
            Some((false, file)) if *file != pos.file => {
                Some((spg_parser::ast::display_name(ns).to_string(), spg_parser::ast::display_name(member).to_string()))
            }
            _ => None,
        }
    }

    fn lookup_var(&self, name: &str) -> Option<&VarInfo> {
        for scope in self.scopes.iter().rev() {
            if let Some(info) = scope.get(name) {
                return Some(info);
            }
        }
        None
    }

    /// records `name = ...` on the innermost binding of `name`
    fn mark_assigned(&mut self, name: &str) {
        for scope in self.scopes.iter_mut().rev() {
            if let Some(info) = scope.get_mut(name) {
                info.assigned = true;
                return;
            }
        }
    }

    /// inside the current scope, `names` (optionals a condition proved aren't
    /// `none`) get their inner type. the entry is a shadow of the real binding
    /// (or replaces it, when both live in this scope). either way, a later
    /// assignment ends it again (`unnarrow`). the narrowing gets forgotten.
    fn narrow(&mut self, names: &[String]) {
        for name in names {
            let found = self.scopes.iter().enumerate().rev().find_map(|(i, s)| s.get(name).map(|info| (i, info.clone())));
            let Some((scope, info)) = found else { continue };
            let Ty::Optional(inner) = &info.ty else { continue };
            let narrowed = VarInfo {
                ty: inner.as_ref().clone(),
                mutable: info.mutable,
                assigned: info.assigned,
                origin: Some(info.origin.unwrap_or(scope)),
                narrowed_from: Some(info.ty.clone()),
            };
            self.scopes.last_mut().expect("at least one scope always active").insert(name.clone(), narrowed);
        }
    }

    /// `when (type(x)) { "int" { ... } }`: inside that arm `x` has the member the arm names. the same
    /// shadow `narrow` makes for an optional, for a member of a union. no assignment is needed to undo
    /// it: the arm's scope goes away with the shadow.
    fn narrow_member(&mut self, name: &str, member: Ty) {
        let found = self.scopes.iter().enumerate().rev().find_map(|(i, s)| s.get(name).map(|info| (i, info.clone())));
        let Some((scope, info)) = found else { return };
        let Ty::Union(members) = &info.ty else { return };
        // a member, or a smaller union of members (what a chain of `else`s has left over)
        let fits = match &member {
            Ty::Union(sub) => sub.iter().all(|t| members.contains(t)) && sub.len() < members.len(),
            t => members.contains(t),
        };
        if !fits {
            return;
        }
        let narrowed = VarInfo {
            ty: member,
            mutable: info.mutable,
            assigned: info.assigned,
            origin: Some(info.origin.unwrap_or(scope)),
            narrowed_from: Some(info.ty.clone()),
        };
        self.scopes.last_mut().expect("at least one scope always active").insert(name.to_string(), narrowed);
    }

    /// applies the facts of `type_facts`, in the current scope. `Not` narrows to the member left
    /// once every excluded member is taken out of the union.
    fn narrow_types(&mut self, facts: &[TypeFact]) {
        for fact in facts {
            match fact {
                TypeFact::Is(name, ty) => self.narrow_member(name, ty.clone()),
                TypeFact::Not(name, _) => {
                    let Some(Ty::Union(members)) = self.resolve_name(name) else { continue };
                    let left: Vec<Ty> = members
                        .iter()
                        .filter(|m| !facts.iter().any(|f| matches!(f, TypeFact::Not(n, t) if n == name && t == *m)))
                        .cloned()
                        .collect();
                    // one member left: the variable is it. several left: the rest of the union, so
                    // the next `else` in the chain still sees what is excluded
                    match left.as_slice() {
                        [only] => self.narrow_member(name, only.clone()),
                        [_, _, ..] if left.len() < members.len() => self.narrow_member(name, Ty::Union(left)),
                        _ => {}
                    }
                }
            }
        }
    }

    /// undoes `narrow` for `name`: the shadow disappears, or the real binding gets
    /// its optional type back. welcome back, `?`.
    fn unnarrow(&mut self, name: &str) {
        let Some(idx) = self.scopes.iter().rposition(|s| s.contains_key(name)) else { return };
        let info = self.scopes[idx].get(name).cloned().expect("found just above");
        let Some(original) = info.narrowed_from else { return };
        if info.origin == Some(idx) {
            let entry = self.scopes[idx].get_mut(name).expect("found just above");
            entry.ty = original;
            entry.narrowed_from = None;
            entry.origin = None;
        } else {
            self.scopes[idx].remove(name);
        }
    }

    /// `left ?? right`
    fn coalesce(&mut self, left: TypedExpr, right: TypedExpr, pos: &p::Pos) -> Option<TypedExpr> {
        let Ty::Optional(inner) = left.ty().clone() else {
            self.errors.push(TypeError::CoalesceNeedsOptional { found: left.ty().clone(), pos: pos.clone() });
            return None;
        };
        let inner = *inner;
        let ty = if inner == *right.ty() {
            inner
        } else if Ty::Optional(Box::new(inner.clone())).accepts(right.ty()) {
            Ty::Optional(Box::new(inner))
        } else {
            self.errors.push(TypeError::InvalidOperandTypes {
                op: "Coalesce".to_string(),
                left: left.ty().clone(),
                right: right.ty().clone(),
                pos: pos.clone(),
            });
            return None;
        };
        Some(TypedExpr::Coalesce { left: Box::new(left), right: Box::new(right), ty })
    }

    /// a call to a builtin (`@str.len(s)`, `print(...)`, ...) whose args
    /// are already checked. `name` must be in the builtin table (callers test
    /// `builtin_signatures().contains_key(..)` first). check before you call.
    ///
    /// builtins get checked against a separate table, so their params can use
    /// `Ty::Any`/`Ty::AnyArray`/`Ty::AnyMap` without messing up FunctionSig. a
    /// name can have several overloads. the first whose arity and arg
    /// types both match wins. simple rule, no drama.
    fn check_builtin_call(&mut self, name: String, typed_args: Vec<TypedExpr>, pos: &p::Pos) -> Option<TypedExpr> {
        let builtins = builtin_signatures();
        let overloads = builtins.get(&name).expect("the caller checked that this is a builtin");
        // `print` / `printError` take ANY number of args (zero
        // included), each of any type. the table's single-`Any`
        // signature is just their name tag. they never fail on arity. free speech.
        if let Some(first) = overloads.first() {
            if matches!(first.id, BuiltinId::Print | BuiltinId::PrintError) {
                for (i, a) in typed_args.iter().enumerate() {
                    if !ty_matches(&Ty::Any, a.ty()) {
                        self.errors.push(TypeError::ArgTypeMismatch {
                            function_name: name.clone(),
                            param_index: i,
                            expected: Ty::Any,
                            found: a.ty().clone(),
                            pos: pos.clone(),
                        });
                    }
                }
                return Some(TypedExpr::Call {
                    callee: Box::new(TypedExpr::Ident { name, ty: Ty::Void }),
                    args: typed_args,
                    ty: Ty::Void,
                    builtin: Some(first.id),
                });
            }
        }
        // `@into.string(color)`: the variant's name
        if name == "@into.string" && typed_args.len() == 1 && self.is_enum_ty(typed_args[0].ty()) {
            return Some(TypedExpr::Call {
                callee: Box::new(TypedExpr::Ident { name, ty: Ty::String }),
                args: typed_args,
                ty: Ty::String,
                builtin: Some(BuiltinId::EnumToString),
            });
        }
        // `@into.string(x)` for any type without its own overload (array, map,
        // struct, ...): the same text `print` shows. consistency, always.
        if name == "@into.string"
            && typed_args.len() == 1
            && !matches!(typed_args[0].ty(), Ty::Int | Ty::Float | Ty::Bool)
        {
            return Some(TypedExpr::Call {
                callee: Box::new(TypedExpr::Ident { name, ty: Ty::String }),
                args: typed_args,
                ty: Ty::String,
                builtin: Some(BuiltinId::AnyToString),
            });
        }
        let arg_types: Vec<&Ty> = typed_args.iter().map(|a| a.ty()).collect();
        let matched = overloads.iter().find(|sig| {
            sig.params.len() == arg_types.len()
                && sig.params.iter().zip(arg_types.iter()).all(|(p, a)| ty_matches(p, a))
        });

        return match matched {
            Some(sig) => {
                // a builtin that returns AnyArray/Any
                // (pop, sort, reverse, slice, ...) figures out its
                // real return type from the matched arg
                // instead of staying a wildcard. a wildcard
                // return type would fail every `let x: T = ...`
                // at the call site, since T is never literally
                // "AnyArray". the first AnyArray/Any param
                // that matched a real arg tells us what T is.
                let resolved_return = if sig.id.is_map_builtin() {
                    self.check_map_call(sig.id, &name, &arg_types, pos)?
                } else if sig.id.is_array_checked() {
                    self.check_array_call(sig.id, &name, &arg_types, pos)?
                } else if sig.id == BuiltinId::TaskAll {
                    self.check_task_all(&name, &arg_types, pos)?
                } else {
                    resolve_wildcard_return(&sig.return_type, &sig.params, &arg_types)
                };
                Some(TypedExpr::Call {
                    callee: Box::new(TypedExpr::Ident { name, ty: resolved_return.clone() }),
                    args: typed_args,
                    ty: resolved_return,
                    builtin: Some(sig.id),
                })
            }
            None => {
                // no overload matched. report against the
                // first overload's arity, the most useful
                // single diagnostic (like how a normal
                // function reports ArityMismatch/ArgTypeMismatch). consistent, as always.
                let first = &overloads[0];
                if first.params.len() != arg_types.len() {
                    self.errors.push(TypeError::ArityMismatch {
                        function_name: name.clone(),
                        expected: first.params.len(),
                        found: arg_types.len(),
                        pos: pos.clone(),
                    });
                } else {
                    for (i, (p, a)) in first.params.iter().zip(arg_types.iter()).enumerate() {
                        if !ty_matches(p, a) {
                            self.errors.push(TypeError::ArgTypeMismatch {
                                function_name: name.clone(),
                                param_index: i,
                                expected: p.clone(),
                                found: (*a).clone(),
                                pos: pos.clone(),
                            });
                        }
                    }
                }
                None
            }
        };
    }

    /// `recv.method(args)` where `recv` is a string, array or map. same as
    /// `@ns.method(recv, args)`, checked by the same builtin rules (so
    /// every overload and type check of the plain call applies). `recv` is
    /// arg #0, evaluated first and once. arity and arg errors
    /// count only the args the caller wrote, never the receiver.
    fn check_native_method(
        &mut self,
        receiver: TypedExpr,
        ns: &str,
        method: &str,
        args: &[Expr],
        pos: &p::Pos,
    ) -> Option<TypedExpr> {
        let name = format!("@{}.{}", ns, method);
        // only builtins whose FIRST param takes the receiver are methods
        // (`@array.range(a, b)` and friends that start with an int are NOT methods. yes.)
        let takes_receiver = builtin_signatures()
            .get(&name)
            .map_or(false, |overloads| overloads.iter().any(|sig| sig.params.first().map_or(false, |p| ty_matches(p, receiver.ty()))));
        if !takes_receiver {
            self.errors.push(TypeError::NoSuchNativeMethod {
                found: receiver.ty().clone(),
                method: method.to_string(),
                pos: pos.clone(),
            });
            return None;
        }
        let mut typed_args = vec![receiver];
        for arg in args {
            typed_args.push(self.check_expr(arg)?);
        }
        let first_new_error = self.errors.len();
        let result = self.check_builtin_call(name, typed_args, pos);
        for e in &mut self.errors[first_new_error..] {
            match e {
                TypeError::ArityMismatch { expected, found, .. } => {
                    *expected = expected.saturating_sub(1);
                    *found = found.saturating_sub(1);
                }
                TypeError::ArgTypeMismatch { param_index, .. } => *param_index = param_index.saturating_sub(1),
                _ => {}
            }
        }
        result
    }

    /// `obj.method(args)` -> the plain call `Struct.method(obj, args)`.
    ///
    /// a method is a normal function the parser registered as
    /// `<Struct>.<method>` with `me: Struct` as its first param, so the
    /// the whole feature is this lookup plus putting the receiver first. no new
    /// typed node, no opcode. `obj` is evaluated first and exactly once (it's
    /// just arg #0). arity and arg errors count only the args the
    /// caller wrote, never `me`. `me` is free. it's always there.
    /// the compiled name of the value method `type.method` that a call at `pos` may use:
    /// an exported one anywhere, or a private one declared in the caller's own file
    fn visible_value_method(&self, type_name: &str, method: &str, pos: &p::Pos) -> Option<String> {
        self.value_methods
            .get(&format!("{}.{}", type_name, method))?
            .iter()
            .find(|m| m.exported || m.file == pos.file)
            .map(|m| m.internal.clone())
    }

    fn check_method_call(&mut self, object: &Expr, method: &str, args: &[Expr], pos: &p::Pos) -> Option<TypedExpr> {
        // `Type.name(args)`: a call to an associated function. the object is a struct NAME,
        // not a value. a local variable with that name wins (then it's a plain method call)
        if let Expr::Ident(type_name, _) = object {
            // `Foo.bar()` where `Foo` is not a struct, and a `function Foo.x` was already
            // reported as an unknown type: the declaration error is the one to show, so
            // the name is not also reported as an undefined variable here
            let declared_for_unknown = !self.is_type_name(type_name)
                && self.lookup_var(type_name).is_none()
                && self.associated.iter().any(|f| f.split_once('.').map_or(false, |(t, _)| t == type_name));
            if declared_for_unknown {
                return None;
            }
            // `Cor.Vermelho()` on an enum is a variant, not a static call: it keeps its own path
            let is_variant = self.enum_defs.get(type_name).map_or(false, |vs| vs.iter().any(|v| v == method));
            if self.lookup_var(type_name).is_none() && self.is_type_name(type_name) && !is_variant {
                let full = format!("{}.{}", type_name, method);
                if !self.associated.contains(&full) {
                    self.errors.push(TypeError::NoAssociatedFunction {
                        type_name: type_name.clone(),
                        name: method.to_string(),
                        pos: pos.clone(),
                    });
                    return None;
                }
                if let Some((namespace, member)) = self.namespace_private(&full, pos) {
                    self.errors.push(TypeError::NamespaceMemberPrivate { namespace, member, pos: pos.clone() });
                    return None;
                }
                let call = Expr::Call {
                    callee: Box::new(Expr::Ident(full, pos.clone())),
                    args: args.to_vec(),
                    pos: pos.clone(),
                };
                return self.check_expr(&call);
            }
        }
        let typed_object = self.check_expr(object)?;
        // a method of a native object is a call of the module's function, with the object first
        if let Some((module, native)) = self.native_object_of(typed_object.ty()) {
            let Some(&(_, function)) = native.methods.iter().find(|(name, _)| *name == method) else {
                self.errors.push(TypeError::NoSuchMethod {
                    struct_name: native.name.to_string(),
                    method: method.to_string(),
                    is_field: native.fields.iter().any(|f| f.name == method),
                    pos: pos.clone(),
                });
                return None;
            };
            let mut typed_args = vec![typed_object];
            for arg in args {
                typed_args.push(self.check_expr(arg)?);
            }
            return self.lib_call(module, function, typed_args, pos);
        }
        // an associated function is not a method: `p.name()` is refused, whatever `p` is
        if let Ty::Named(type_name) = typed_object.ty() {
            if self.associated.contains(&format!("{}.{}", type_name, method)) {
                self.errors.push(TypeError::AssociatedCalledOnValue {
                    type_name: type_name.clone(),
                    name: method.to_string(),
                    pos: pos.clone(),
                });
                return None;
            }
        }
        // `x.string()`, `x.int()`, `x.float()`: shorthand for `@into.string(x)` etc. on
        // any value. a struct's own method with the same name wins
        if matches!(method, "string" | "int" | "float") && args.is_empty() {
            let user_method = match typed_object.ty() {
                Ty::Named(n) => self.functions.contains_key(&format!("{}.{}", n, method)),
                _ => false,
            };
            if !user_method {
                // already of the target type: the conversion is the value itself (`2.5.float()`)
                let same = match method {
                    "int" => Ty::Int,
                    "float" => Ty::Float,
                    _ => Ty::String,
                };
                if *typed_object.ty() == same {
                    return Some(typed_object);
                }
                return self.check_builtin_call(format!("@into.{}", method), vec![typed_object], pos);
            }
        }
        if self.is_enum_ty(typed_object.ty()) {
            self.errors.push(TypeError::NoMethodsOnType {
                found: typed_object.ty().clone(),
                method: method.to_string(),
                pos: pos.clone(),
            });
            return None;
        }
        // `x.copy()`: a deep, mutable copy of an array, map or struct, typed as the receiver.
        // a struct's own `copy` method wins, like any other method name
        if method == "copy" && args.is_empty() {
            let user_method = match typed_object.ty() {
                Ty::Named(n) => self.functions.contains_key(&format!("{}.copy", n)),
                _ => false,
            };
            if !user_method && matches!(typed_object.ty(), Ty::Array(_) | Ty::Map(..) | Ty::Named(_)) {
                let ty = typed_object.ty().clone();
                return Some(TypedExpr::Call {
                    callee: Box::new(TypedExpr::Ident { name: "@copy".to_string(), ty: Ty::Void }),
                    args: vec![typed_object],
                    ty,
                    builtin: Some(BuiltinId::ValueCopy),
                });
            }
        }
        // `t.wait()`: the value of a task. the result type is the task's own T
        if let (Ty::Task(inner), "wait", true) = (typed_object.ty().clone(), method, args.is_empty()) {
            return Some(TypedExpr::Call {
                callee: Box::new(TypedExpr::Ident { name: "@task.wait".to_string(), ty: Ty::Void }),
                args: vec![typed_object],
                ty: *inner,
                builtin: Some(BuiltinId::TaskWait),
            });
        }
        // `s.len()`, `xs.push(1)`, `m.has(k)`, `n.abs()`, `f.sqrt()`: the methods of a
        // value are the builtins of the method namespaces that take it first
        let receiver_ty = typed_object.ty().clone();
        // a user method on a value type (`fn string.shout(s: string)`): it is a plain call with
        // the value first. its name cannot clash with a built-in (checked at its declaration)
        let value_method = value_type_of_ty(&receiver_ty)
            .and_then(|t| self.visible_value_method(t, method, pos).map(|internal| (t, internal)));
        let value_internal = value_method.as_ref().map(|(_, internal)| internal.clone());
        if value_method.is_none() {
            if let Some(ns) = method_namespace(&receiver_ty, method) {
                return self.check_native_method(typed_object, ns, method, args, pos);
            }
            // a method of this value type that exists but is not visible here: another file's, not exported
            if let Some(type_name) = value_type_of_ty(&receiver_ty) {
                let key = format!("{}.{}", type_name, method);
                if self.value_methods.get(&key).map_or(false, |ms| !ms.is_empty()) {
                    self.errors.push(TypeError::ValueMethodNotVisible {
                        type_name: type_name.to_string(),
                        method: method.to_string(),
                        pos: pos.clone(),
                    });
                    return None;
                }
            }
            if matches!(receiver_ty, Ty::String | Ty::Array(_) | Ty::Map(..) | Ty::Int | Ty::Float) {
                self.errors.push(TypeError::NoSuchNativeMethod {
                    found: receiver_ty,
                    method: method.to_string(),
                    pos: pos.clone(),
                });
                return None;
            }
        }
        let struct_name = match (value_method, typed_object.ty()) {
            (Some((t, _)), _) => t.to_string(),
            (None, Ty::Named(n)) => n.clone(),
            (None, other) => {
                self.errors.push(TypeError::NoMethodsOnType {
                    found: other.clone(),
                    method: method.to_string(),
                    pos: pos.clone(),
                });
                return None;
            }
        };

        // the method's declared params (after `self`) tell each argument its expected type
        let full_name = value_internal.unwrap_or_else(|| format!("{}.{}", struct_name, method));
        let method_params: Vec<Ty> = self
            .functions
            .get(&full_name)
            .map(|sig| sig.params.iter().skip(1).map(|p| resolve_type_expr(&p.type_ann)).collect())
            .unwrap_or_default();
        let mut typed_args = vec![typed_object];
        for (i, arg) in args.iter().enumerate() {
            typed_args.push(self.check_expr_expecting(arg, method_params.get(i))?);
        }
        let (expected_params, expected_return, method_defaults) = match self.functions.get(&full_name).cloned() {
            Some(sig) => (
                sig.params.iter().skip(1).map(|p| resolve_type_expr(&p.type_ann)).collect::<Vec<Ty>>(),
                sig.return_type.clone(),
                sig.params.iter().skip(1).map(|p| p.default.clone()).collect::<Vec<Option<p::Expr>>>(),
            ),
            None => {
                let is_field = self
                    .struct_defs
                    .get(&struct_name)
                    .map_or(false, |fields| fields.iter().any(|f| f.name == method));
                self.errors.push(TypeError::NoSuchMethod {
                    struct_name,
                    method: method.to_string(),
                    is_field,
                    pos: pos.clone(),
                });
                return None;
            }
        };

        let given = typed_args.len() - 1;
        let required = method_defaults.iter().filter(|d| d.is_none()).count();
        if given < required || given > expected_params.len() {
            self.errors.push(TypeError::ArityMismatch {
                function_name: full_name,
                expected: expected_params.len(),
                found: given,
                pos: pos.clone(),
            });
            return None;
        }
        for (i, (arg, expected)) in typed_args.iter().skip(1).zip(expected_params.iter()).enumerate() {
            if !ty_matches(expected, arg.ty()) {
                self.errors.push(TypeError::ArgTypeMismatch {
                    function_name: full_name.clone(),
                    param_index: i,
                    expected: expected.clone(),
                    found: arg.ty().clone(),
                    pos: pos.clone(),
                });
            }
        }
        // left-out args take the method's default values (same rule as functions)
        for i in given..expected_params.len() {
            let Some(default) = method_defaults[i].as_ref() else { break };
            let Some(typed_default) = self.check_expr(default) else { return None };
            if !ty_matches(&expected_params[i], typed_default.ty()) {
                self.errors.push(TypeError::ArgTypeMismatch {
                    function_name: full_name.clone(),
                    param_index: i,
                    expected: expected_params[i].clone(),
                    found: typed_default.ty().clone(),
                    pos: pos.clone(),
                });
            }
            typed_args.push(typed_default);
        }

        Some(TypedExpr::Call {
            callee: Box::new(TypedExpr::Ident { name: full_name, ty: expected_return.clone() }),
            args: typed_args,
            ty: expected_return,
            builtin: None,
        })
    }

    // statements (the bulk of it)

    /// the guarded struct that `object` (the target of a field write) is, if any.
    /// the type is probed with its errors dropped: the real check of the statement
    /// reports them. `$`-named temporaries are already the wrapped form of a write.
    fn guarded_object(&mut self, object: &Expr) -> Option<String> {
        if self.guarded_structs.is_empty() {
            return None;
        }
        if let Expr::Ident(n, _) = object {
            if n.starts_with('$') {
                return None;
            }
        }
        let before = self.errors.len();
        let typed = self.check_expr(object);
        self.errors.truncate(before);
        let ty = typed?.ty().clone();
        match ty {
            Ty::Named(name) if self.guarded_structs.contains(&name) => Some(name),
            _ => None,
        }
    }

    /// `if (true) {
    ///   let $o = object; let $old = $o.field; <write on $o>;
    ///   let $d = @guard.enter("S");           // running guards of S in this task, before us
    ///   try {
    ///     if ($d > 0) { throw "..." }         // a guard of S is already running: refuse
    ///     $o.__guard()
    ///   } catch ($err) {
    ///     @guard.leave("S"); $o.field = $old; throw $err
    ///   }
    ///   @guard.leave("S")
    /// }`
    /// the write runs first, then the guards; a refusal puts the old value back.
    /// enter and leave always pair up, on the success and the refusal path.
    fn wrap_guarded_write(&self, struct_name: &str, object: &Expr, field: &str, pos: &p::Pos, write: &dyn Fn(Expr) -> Stmt) -> Stmt {
        let tmp = format!("$guard_obj_{}_{}", pos.line, pos.col);
        let old = format!("$guard_old_{}_{}", pos.line, pos.col);
        let depth = format!("$guard_depth_{}_{}", pos.line, pos.col);
        let err = format!("$guard_err_{}_{}", pos.line, pos.col);
        let var = |n: &str| Expr::Ident(n.to_string(), pos.clone());
        let call = |name: &str, args: Vec<Expr>| Expr::Call {
            callee: Box::new(Expr::Ident(name.to_string(), pos.clone())),
            args,
            pos: pos.clone(),
        };
        let name_arg = || Expr::StringLit(struct_name.to_string(), pos.clone());
        let stmts = vec![
            Stmt::Let { name: tmp.clone(), type_ann: None, value: object.clone(), exported: false, pos: pos.clone() },
            Stmt::Let {
                name: old.clone(),
                type_ann: None,
                value: Expr::Field { object: Box::new(var(&tmp)), field: field.to_string(), pos: pos.clone() },
                exported: false,
                pos: pos.clone(),
            },
            write(var(&tmp)),
            Stmt::Let {
                name: depth.clone(),
                type_ann: None,
                value: call("@guard.enter", vec![name_arg()]),
                exported: false,
                pos: pos.clone(),
            },
            Stmt::Try {
                try_block: p::Block {
                    stmts: vec![
                        Stmt::IfStmt {
                            cond: Expr::Binary {
                                left: Box::new(var(&depth)),
                                op: BinaryOp::Gt,
                                right: Box::new(Expr::IntLit(0, pos.clone())),
                                pos: pos.clone(),
                            },
                            then_branch: p::Block {
                                stmts: vec![Stmt::Throw {
                                    value: Expr::StringLit(
                                        format!("a write to a {} ran inside a guard of {}: a guard cannot change its own struct", struct_name, struct_name),
                                        pos.clone(),
                                    ),
                                    pos: pos.clone(),
                                }],
                                pos: pos.clone(),
                            },
                            else_branch: None,
                            pos: pos.clone(),
                        },
                        Stmt::ExprStmt(Expr::Call {
                            callee: Box::new(Expr::Field { object: Box::new(var(&tmp)), field: "__guard".to_string(), pos: pos.clone() }),
                            args: Vec::new(),
                            pos: pos.clone(),
                        }),
                    ],
                    pos: pos.clone(),
                },
                catch_var: err.clone(),
                catch_type: None,
                catch_block: p::Block {
                    stmts: vec![
                        Stmt::ExprStmt(call("@guard.leave", vec![name_arg()])),
                        Stmt::FieldAssign {
                            object: var(&tmp),
                            field: field.to_string(),
                            value: var(&old),
                            pos: pos.clone(),
                        },
                        Stmt::Throw { value: var(&err), pos: pos.clone() },
                    ],
                    pos: pos.clone(),
                },
                pos: pos.clone(),
            },
            Stmt::ExprStmt(call("@guard.leave", vec![name_arg()])),
        ];
        Stmt::IfStmt {
            cond: Expr::BoolLit(true, pos.clone()),
            then_branch: p::Block { stmts, pos: pos.clone() },
            else_branch: None,
            pos: pos.clone(),
        }
    }

    fn check_stmt(&mut self, stmt: &Stmt) -> Option<TypedStmt> {
        match stmt {
            Stmt::Let { name, type_ann, value, exported, pos } => {
                let hint = type_ann.as_ref().map(resolve_type_expr);
                let typed_value = self.check_expr_expecting(value, hint.as_ref())?;
                let inferred = typed_value.ty().clone();
                if inferred == Ty::Void {
                    self.errors.push(TypeError::VoidValue { pos: pos.clone() });
                }

                let final_ty = if let Some(ann) = type_ann {
                    let declared = resolve_type_expr(ann);
                    if !declared.accepts(&inferred) {
                        self.errors.push(TypeError::AnnotationMismatch {
                            expected: declared.clone(),
                            found: inferred,
                            pos: pos.clone(),
                        });
                    }
                    declared
                } else {
                    if inferred.is_empty_map() {
                        self.errors.push(TypeError::EmptyMapNeedsType { pos: pos.clone() });
                    }
                    if inferred == Ty::NoneLit {
                        self.errors.push(TypeError::NoneNeedsType { pos: pos.clone() });
                    }
                    inferred
                };

                self.declare(name, final_ty.clone(), true, pos);
                Some(TypedStmt::Let { name: name.clone(), ty: final_ty, value: typed_value, exported: *exported })
            }

            Stmt::Const { name, type_ann, value, exported, pos } => {
                let typed_value = self.check_expr(value)?;
                let inferred = typed_value.ty().clone();
                if inferred == Ty::Void {
                    self.errors.push(TypeError::VoidValue { pos: pos.clone() });
                }

                let final_ty = if let Some(ann) = type_ann {
                    let declared = resolve_type_expr(ann);
                    if !declared.accepts(&inferred) {
                        self.errors.push(TypeError::AnnotationMismatch {
                            expected: declared.clone(),
                            found: inferred,
                            pos: pos.clone(),
                        });
                    }
                    declared
                } else {
                    if inferred.is_empty_map() {
                        self.errors.push(TypeError::EmptyMapNeedsType { pos: pos.clone() });
                    }
                    if inferred == Ty::NoneLit {
                        self.errors.push(TypeError::NoneNeedsType { pos: pos.clone() });
                    }
                    inferred
                };

                self.declare(name, final_ty.clone(), false, pos);
                Some(TypedStmt::Const { name: name.clone(), ty: final_ty, value: typed_value, exported: *exported })
            }

            Stmt::Match { subject, arms, else_branch, pos } => {
                let typed_subject = self.check_expr(subject)?;
                let subject_ty = typed_subject.ty().clone();

                // enum subjects carry their variant names. int and string ones have none. sad.
                let variants: Option<Vec<String>> = match &subject_ty {
                    Ty::Named(n) => self.enum_named(n).cloned(),
                    _ => None,
                };
                if variants.is_none() && !matches!(subject_ty, Ty::Int | Ty::String) {
                    self.errors.push(TypeError::MatchSubjectType { found: subject_ty.clone(), pos: pos.clone() });
                    return None;
                }

                let mut typed_arms: Vec<(Vec<TypedExpr>, TypedBlock)> = Vec::new();
                let mut covered: Vec<usize> = Vec::new();
                let mut bad_case = false;
                let mut type_covered: Vec<Ty> = Vec::new();
                let mut seen_literals: Vec<String> = Vec::new();
                for arm in arms {
                    let mut values = Vec::new();
                    for value in &arm.values {
                        let typed_value = self.check_expr(value)?;
                        // the first matching case wins, so a repeat can never run. dead code, friend.
                        let key = match &typed_value {
                            TypedExpr::IntLit(n) => Some(format!("{}", n)),
                            TypedExpr::StringLit(st) => Some(format!("{:?}", st)),
                            TypedExpr::EnumLit { index, .. } => variants.as_ref().map(|v| v[*index].clone()),
                            _ => None,
                        };
                        if let Some(k) = key {
                            if seen_literals.contains(&k) {
                                self.errors.push(TypeError::MatchDuplicateCase { value: k.clone(), pos: arm.pos.clone() });
                            } else {
                                seen_literals.push(k);
                            }
                        }
                        if typed_value.ty() != &subject_ty {
                            self.errors.push(TypeError::MatchCaseType {
                                expected: subject_ty.clone(),
                                found: typed_value.ty().clone(),
                                pos: arm.pos.clone(),
                            });
                            bad_case = true;
                            continue;
                        }
                        if let TypedExpr::EnumLit { index, .. } = &typed_value {
                            covered.push(*index);
                        }
                        values.push(typed_value);
                    }
                    self.push_scope();
                    if let Some((var, member)) = type_test_arm(subject, &arm.values) {
                        type_covered.push(member.clone());
                        self.narrow_member(&var, member);
                    }
                    let typed_body = self.check_block(&arm.body);
                    self.pop_scope();
                    typed_arms.push((values, typed_body));
                }

                let typed_else = else_branch.as_ref().map(|b| {
                    self.push_scope();
                    let t = self.check_block(b);
                    self.pop_scope();
                    t
                });

                // `when (type(x))` without `else` is exhaustive when its arms name every member of x's union
                if let (None, false) = (&typed_else, bad_case) {
                    if let Some(var) = type_test_subject(subject) {
                        if let Some(Ty::Union(members)) = self.resolve_name(&var) {
                            if members.iter().all(|m| type_covered.contains(m)) {
                                self.exhaustive_matches.insert((pos.file.clone(), pos.line, pos.col));
                            }
                        }
                    }
                }

                // an enum match without `else` has to name every variant. no skipping.
                if let (Some(names), None, false) = (&variants, &typed_else, bad_case) {
                    let missing: Vec<String> = names
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| !covered.contains(i))
                        .map(|(_, n)| n.clone())
                        .collect();
                    if !missing.is_empty() {
                        self.errors.push(TypeError::MatchNonExhaustive { missing, pos: pos.clone() });
                    } else {
                        self.exhaustive_matches.insert((pos.file.clone(), pos.line, pos.col));
                    }
                }

                // lowering: `let $match = subject;` then an if/elif chain comparing
                // against it (enums compare as their int index, like `==` does). sneaky.
                let hidden = "$match".to_string();
                let is_enum = variants.is_some();
                let wrap = |e: TypedExpr| if is_enum { TypedExpr::AsInt(Box::new(e)) } else { e };
                let subject_ref = TypedExpr::Ident { name: hidden.clone(), ty: subject_ty.clone() };
                let loc = SrcLoc { file: pos.file.clone(), line: pos.line };

                let mut chain: Option<TypedBlock> = typed_else;
                for (values, body) in typed_arms.into_iter().rev() {
                    let cond = values
                        .into_iter()
                        .map(|v| TypedExpr::Binary {
                            left: Box::new(wrap(subject_ref.clone())),
                            op: BinaryOp::Eq,
                            right: Box::new(wrap(v)),
                            ty: Ty::Bool,
                        })
                        .reduce(|a, b| TypedExpr::Binary { left: Box::new(a), op: BinaryOp::Or, right: Box::new(b), ty: Ty::Bool });
                    if let Some(cond) = cond {
                        chain = Some(TypedBlock {
                            stmts: vec![TypedStmt::IfStmt { cond, then_branch: body, else_branch: chain }],
                            locs: vec![loc.clone()],
                            ty: Ty::Void,
                        });
                    }
                }

                let mut stmts = vec![TypedStmt::Let { name: hidden, ty: subject_ty, value: typed_subject, exported: false }];
                let mut locs = vec![loc.clone()];
                if let Some(c) = chain {
                    stmts.extend(c.stmts);
                    locs.extend(c.locs);
                }
                Some(TypedStmt::Block { body: TypedBlock { stmts, locs, ty: Ty::Void } })
            }

            Stmt::IfStmt { cond, then_branch, else_branch, pos } => {
                let typed_cond = self.check_expr(cond)?;
                if typed_cond.ty() != &Ty::Bool {
                    self.errors.push(TypeError::NonBoolCondition { found: typed_cond.ty().clone(), pos: pos.clone() });
                }

                let (when_true, when_false) = narrowing(cond);
                let (type_true, type_false) = type_facts(cond);

                self.push_scope();
                self.narrow(&when_true);
                self.narrow_types(&type_true);
                let typed_then = self.check_block(then_branch);
                self.pop_scope();

                let typed_else = else_branch.as_ref().map(|b| {
                    self.push_scope();
                    self.narrow(&when_false);
                    self.narrow_types(&type_false);
                    let t = self.check_block(b);
                    self.pop_scope();
                    t
                });

                // a branch that always leaves (return, throw, break, continue) means the
                // OTHER side of the condition holds for everything after the `if`. logic, babe.
                let then_exits = block_exits(then_branch);
                let else_exits = else_branch.as_ref().is_some_and(|b| block_exits(b));
                if then_exits && !else_exits {
                    self.narrow(&when_false);
                    self.narrow_types(&type_false);
                } else if else_exits && !then_exits {
                    self.narrow(&when_true);
                    self.narrow_types(&type_true);
                }

                Some(TypedStmt::IfStmt { cond: typed_cond, then_branch: typed_then, else_branch: typed_else })
            }

            // the body is checked before the condition, but it can't see the body's
            // names: the condition is outside the body's scope, like in most languages
            Stmt::DoWhile { body, cond, pos } => {
                self.loop_depth += 1;
                self.push_scope();
                let typed_body = self.check_block(body);
                self.pop_scope();
                self.loop_depth -= 1;
                let typed_cond = self.check_expr(cond)?;
                if typed_cond.ty() != &Ty::Bool {
                    self.errors.push(TypeError::NonBoolCondition { found: typed_cond.ty().clone(), pos: pos.clone() });
                }
                Some(TypedStmt::DoWhile { body: typed_body, cond: typed_cond })
            }

            // `compile VM { }`: an ordinary block. the compiler marks its code as VM-only
            Stmt::CompileVm { body, .. } => {
                // a block scope, like `if`: names declared inside stay inside
                self.push_scope();
                let typed_body = self.check_block(body);
                self.pop_scope();
                Some(TypedStmt::CompileVm { body: typed_body })
            }

            // `compile JIT { }`: an ordinary block too. the compiler records it, and spg-jit checks it
            Stmt::CompileJit { body, .. } => {
                self.push_scope();
                let typed_body = self.check_block(body);
                self.pop_scope();
                Some(TypedStmt::CompileJit { body: typed_body })
            }

            Stmt::While { cond, body, pos } => {
                let typed_cond = self.check_expr(cond)?;
                if typed_cond.ty() != &Ty::Bool {
                    self.errors.push(TypeError::NonBoolCondition { found: typed_cond.ty().clone(), pos: pos.clone() });
                }
                let (when_true, _) = narrowing(cond);
                let (type_true, _) = type_facts(cond);
                self.push_scope();
                self.narrow(&when_true);
                self.narrow_types(&type_true);
                self.loop_depth += 1;
                let typed_body = self.check_block(body);
                self.loop_depth -= 1;
                self.pop_scope();
                Some(TypedStmt::While { cond: typed_cond, body: typed_body })
            }

            Stmt::ForIn { var_name, index_var, iterable, body, pos } => {
                // `for c in Color { }`: the enum's name (not a variable) iterates
                // its variants in declaration order. lowered to a loop over an
                // array literal of the variants. nothing downstream notices. good.
                let typed_iterable = match iterable {
                    Expr::Ident(n, _) if self.enum_named(n).is_some() => {
                        let variants = self.enum_named(n).cloned().unwrap_or_default();
                        let ty = Ty::Named(n.clone());
                        TypedExpr::ArrayLit {
                            items: (0..variants.len())
                                .map(|index| TypedExpr::EnumLit { name: n.clone(), index, ty: ty.clone() })
                                .collect(),
                            ty: Ty::Array(Box::new(ty)),
                        }
                    }
                    _ => self.check_expr(iterable)?,
                };
                // `for c in s`: the characters of s, the same as `s.split("")`. lowered here, so the
                // loop below is the array loop and nothing downstream sees a string
                let typed_iterable = if matches!(typed_iterable.ty(), Ty::String) {
                    TypedExpr::Call {
                        callee: Box::new(TypedExpr::Ident { name: "@str.split".to_string(), ty: Ty::Void }),
                        args: vec![typed_iterable, TypedExpr::StringLit(String::new())],
                        ty: Ty::Array(Box::new(Ty::String)),
                        builtin: Some(BuiltinId::Split),
                    }
                } else {
                    typed_iterable
                };
                // `for k, v in map`: loops over the keys in insertion order, and reads each
                // value from the map. both names are needed, so a map is never a single-name loop
                if let Ty::Map(key_ty, val_ty) = typed_iterable.ty() {
                    let (key_ty, val_ty) = ((**key_ty).clone(), (**val_ty).clone());
                    let index = match index_var {
                        Some(index) => index.clone(),
                        None => {
                            self.errors.push(TypeError::NotIterable { found: typed_iterable.ty().clone(), pos: pos.clone() });
                            return None;
                        }
                    };
                    let keys = TypedExpr::Call {
                        callee: Box::new(TypedExpr::Ident { name: "@map.keys".to_string(), ty: Ty::Void }),
                        args: vec![typed_iterable.clone()],
                        ty: Ty::Array(Box::new(key_ty.clone())),
                        builtin: Some(BuiltinId::MapKeys),
                    };
                    self.push_scope();
                    self.declare(&index, key_ty.clone(), true, pos);
                    self.declare(var_name, val_ty, true, pos);
                    self.loop_depth += 1;
                    let typed_body = self.check_block(body);
                    self.loop_depth -= 1;
                    self.pop_scope();
                    return Some(TypedStmt::ForIn {
                        var_name: var_name.clone(),
                        index_var: Some(index),
                        elem_ty: key_ty,
                        iterable: keys,
                        body: typed_body,
                        map_source: Some(Box::new(typed_iterable)),
                    });
                }

                let elem_ty = match typed_iterable.ty() {
                    Ty::Array(inner) => (**inner).clone(),
                    other => {
                        self.errors.push(TypeError::NotIterable { found: other.clone(), pos: pos.clone() });
                        Ty::Void
                    }
                };

                self.push_scope();
                if let Some(index) = index_var {
                    self.declare(index, Ty::Int, true, pos);
                }
                self.declare(var_name, elem_ty.clone(), true, pos);
                self.loop_depth += 1;
                let typed_body = self.check_block(body);
                self.loop_depth -= 1;
                self.pop_scope();

                Some(TypedStmt::ForIn {
                    var_name: var_name.clone(),
                    index_var: index_var.clone(),
                    elem_ty,
                    iterable: typed_iterable,
                    body: typed_body,
                    map_source: None,
                })
            }

            Stmt::ForRange { var_name, start, end, step, body, pos } => {
                // all three are evaluated once, before the loop, in the
                // enclosing scope. each gets checked even if an earlier one
                // failed, so one run reports every bad part. complaints, all at once.
                let mut typed_start = self.check_expr(start);
                let mut typed_end = self.check_expr(end);
                let typed_step = step.as_ref().map(|s| self.check_expr(s));

                let mut ok = true;
                for (which, e) in [("start", &mut typed_start), ("end", &mut typed_end)] {
                    if let Some(te) = e {
                        if *te.ty() != Ty::Int {
                            self.errors.push(TypeError::RangeNotInt {
                                which,
                                found: te.ty().clone(),
                                pos: pos.clone(),
                            });
                            ok = false;
                        }
                    } else {
                        ok = false;
                    }
                }

                let range_step = match (&typed_step, step) {
                    (None, _) => Some(RangeStep::One),
                    (Some(None), _) => {
                        ok = false;
                        None
                    }
                    (Some(Some(te)), Some(src)) => {
                        if *te.ty() != Ty::Int {
                            self.errors.push(TypeError::RangeNotInt {
                                which: "step",
                                found: te.ty().clone(),
                                pos: pos.clone(),
                            });
                            ok = false;
                            None
                        } else if let Some(n) = literal_int(src) {
                            if n == 0 {
                                self.errors.push(TypeError::RangeZeroStep { pos: pos.clone() });
                                ok = false;
                                None
                            } else {
                                Some(RangeStep::Const(n))
                            }
                        } else {
                            Some(RangeStep::Dynamic(Box::new(te.clone())))
                        }
                    }
                    _ => None,
                };

                self.push_scope();
                self.declare(var_name, Ty::Int, true, pos);
                self.loop_depth += 1;
                let typed_body = self.check_block(body);
                self.loop_depth -= 1;
                let var_assigned = self.scopes.last().and_then(|sc| sc.get(var_name.as_str())).map_or(true, |v| v.assigned);
                self.pop_scope();

                if !ok {
                    return None;
                }
                Some(TypedStmt::ForRange {
                    var_name: var_name.clone(),
                    start: typed_start?,
                    end: typed_end?,
                    step: range_step?,
                    var_assigned,
                    body: typed_body,
                })
            }

            Stmt::Return { value, pos } => {
                let typed_value = match value {
                    Some(v) => {
                        let hint = self.current_return.clone();
                        Some(self.check_expr_expecting(v, hint.as_ref())?)
                    }
                    None => None,
                };
                // compared here, in the same pass that checked the
                // expression (see `current_return`). skipped when the
                // function's inference already failed. that was reported
                // as CannotInferReturnType, and comparing to the
                // Void placeholder would only add noise. redundant noise. ugh.
                if let Some(expected) = self.current_return.clone() {
                    let found = typed_value.as_ref().map(|v| v.ty().clone()).unwrap_or(Ty::Void);
                    if !expected.accepts(&found) {
                        self.errors.push(TypeError::ReturnTypeMismatch {
                            expected,
                            found,
                            pos: pos.clone(),
                        });
                    }
                }
                Some(TypedStmt::Return { value: typed_value })
            }

            Stmt::Throw { value, pos } => {
                let typed_value = self.check_expr(value)?;
                if *typed_value.ty() != Ty::String {
                    self.errors.push(TypeError::ThrowNotString { found: typed_value.ty().clone(), pos: pos.clone() });
                    return None;
                }
                Some(TypedStmt::Throw { value: typed_value })
            }

            Stmt::Try { try_block, catch_var, catch_type, catch_block, pos } => {
                self.push_scope();
                let typed_try = self.check_block(try_block);
                self.pop_scope();

                // what a `catch` gets is the thrown STRING (the message of a
                // `throw`, or the text of a runtime error, like a div by zero)
                let catch_ty = Ty::String;
                if let Some(ann) = catch_type {
                    let declared = resolve_type_expr(ann);
                    if declared != Ty::String {
                        self.errors.push(TypeError::CatchNotString { found: declared, pos: pos.clone() });
                    }
                }

                self.push_scope();
                self.declare(catch_var, catch_ty.clone(), true, pos);
                let typed_catch = self.check_block(catch_block);
                self.pop_scope();

                Some(TypedStmt::Try {
                    try_block: typed_try,
                    catch_var: catch_var.clone(),
                    catch_ty,
                    catch_block: typed_catch,
                })
            }

            Stmt::FunctionDecl { name, params, return_type, body, exported, pos } => {
                // the real return type was already resolved during
                // collect_function_signatures. for an explicit `-> Type`
                // that's just resolve_type_expr. for an omitted one it's
                // whatever infer_return_type decided. reading it back
                // from `self.functions` avoids inferring twice (and
                // possibly disagreeing with itself if it did). trust, but verify.
                let expected_return = self
                    .functions
                    .get(name)
                    .map(|sig| sig.return_type.clone())
                    .unwrap_or_else(|| return_type.as_ref().map(resolve_type_expr).unwrap_or(Ty::Void));

                // a `fn som.x` body sees the namespace's statics; params and locals start at this depth
                let saved_namespace = self.namespace_ctx.take();
                if let Some((ns, _)) = name.split_once('.') {
                    self.namespace_ctx = Some((ns.to_string(), self.scopes.len()));
                }
                self.push_scope();
                for p in params {
                    self.declare(&p.name, resolve_type_expr(&p.type_ann), true, pos);
                }

                let saved_loop_depth = self.loop_depth;
                self.loop_depth = 0;
                let saved_return = self.current_return.take();
                // a `-> task<T>` body returns a plain T: the task wrapping happens at call time
                let body_return = match &expected_return {
                    Ty::Task(inner) => inner.as_ref().clone(),
                    other => other.clone(),
                };
                if !self.uninferrable_functions.contains(name) {
                    self.current_return = Some(body_return.clone());
                }
                let typed_body = self.check_block(body);
                self.current_return = saved_return;
                self.check_all_paths_return(&body.stmts, &body_return, name, pos);
                self.loop_depth = saved_loop_depth;

                self.pop_scope();
                self.namespace_ctx = saved_namespace;

                Some(TypedStmt::FunctionDecl {
                    name: name.clone(),
                    params: params.clone(),
                    return_type: expected_return,
                    body: typed_body,
                    exported: *exported,
                })
            }

            Stmt::EnumDecl { name, variants, values, exported, .. } => {
                Some(TypedStmt::EnumDecl {
                    name: name.clone(),
                    variants: variants.clone(),
                    values: values.clone(),
                    exported: *exported,
                })
            }

            Stmt::StructDecl { name, fields, exported, .. } => {
                Some(TypedStmt::StructDecl { name: name.clone(), fields: fields.clone(), exported: *exported })
            }

            Stmt::Use { names, from, .. } => {
                Some(TypedStmt::Use { names: names.clone(), from: from.clone() })
            }

            // consumed by spg-modules along with every `Import`. one
            // reaching the checker means the resolver skipped it. resolver, explain yourself.
            Stmt::ReExport { .. } => {
                unreachable!("compiler bug: `public (..) from` reached the typechecker (spg-modules should have consumed it)")
            }
            Stmt::Load { .. } => {
                unreachable!("compiler bug: `load` reached the typechecker (spg-modules should have consumed it)")
            }

            Stmt::Assign { name, value, pos } => {
                if let Some(full) = self.namespace_member(name) {
                    return self.check_stmt(&Stmt::Assign { name: full, value: value.clone(), pos: pos.clone() });
                }
                let hint = self.lookup_var(name).map(|info| info.ty.clone());
                let typed_value = self.check_expr_expecting(value, hint.as_ref())?;
                let found_ty = typed_value.ty().clone();

                match self.lookup_var(name) {
                    None => {
                        if spg_lexer::is_install_namespace(name.as_str()) {
                        // a bare `http` / `ps` (no `.member`): say what it is instead of "undefined"
                        self.errors.push(TypeError::NeedsLoad { lib: name.to_string(), pos: pos.clone() });
                    } else {
                        self.errors.push(TypeError::UndefinedVariable { name: name.clone(), pos: pos.clone() });
                    }
                        None
                    }
                    Some(_) if self.assigns_capture(name) => {
                        self.errors.push(TypeError::CaptureAssign { name: name.clone(), pos: pos.clone() });
                        None
                    }
                    Some(info) if !info.mutable => {
                        self.errors.push(TypeError::AssignToConst { name: name.clone(), pos: pos.clone() });
                        None
                    }
                    Some(info) => {
                        // while narrowed, the variable can still get anything its real type allows
                        let expected = info.narrowed_from.clone().unwrap_or_else(|| info.ty.clone());
                        let narrowed = info.narrowed_from.is_some();
                        if !expected.accepts(&found_ty) {
                            self.errors.push(TypeError::AssignTypeMismatch {
                                name: name.clone(),
                                expected,
                                found: found_ty,
                                pos: pos.clone(),
                            });
                            return None;
                        }
                        if narrowed {
                            self.unnarrow(name);
                        }
                        self.mark_assigned(name);
                        Some(TypedStmt::Assign { name: name.clone(), value: typed_value })
                    }
                }
            }

            Stmt::IndexAssign { array, index, value, pos } => {
                let typed_array = self.check_expr(array)?;
                let typed_index = self.check_expr(index)?;
                let typed_value = self.check_expr(value)?;

                let elem_ty = self.index_target(typed_array.ty(), typed_index.ty(), pos)?;

                if !elem_ty.accepts(typed_value.ty()) {
                    if matches!(typed_array.ty(), Ty::Map(..)) {
                        self.errors.push(TypeError::MapValueMismatch {
                            expected: elem_ty,
                            found: typed_value.ty().clone(),
                            pos: pos.clone(),
                        });
                    } else {
                        self.errors.push(TypeError::IndexAssignTypeMismatch {
                            expected: elem_ty,
                            found: typed_value.ty().clone(),
                            pos: pos.clone(),
                        });
                    }
                    return None;
                }

                Some(TypedStmt::IndexAssign { array: typed_array, index: typed_index, value: typed_value })
            }

            Stmt::IndexCompoundAssign { array, index, op, value, pos } => {
                let typed_array = self.check_expr(array)?;
                let typed_index = self.check_expr(index)?;
                let typed_value = self.check_expr(value)?;

                let elem_ty = self.index_target(typed_array.ty(), typed_index.ty(), pos)?;

                // `elem OP value` must both typecheck as a binary
                // operation (e.g. no string *= int) and give back the
                // element type. same rule an ordinary `x OP= value`
                // enforces for a local, just checked against elem_ty
                // instead of a variable's declared type. close enough.
                let result_ty = self.binary_result_type(op, &elem_ty, typed_value.ty(), pos)?;
                if !elem_ty.accepts(&result_ty) {
                    self.errors.push(TypeError::IndexAssignTypeMismatch {
                        expected: elem_ty,
                        found: result_ty,
                        pos: pos.clone(),
                    });
                    return None;
                }

                Some(TypedStmt::IndexCompoundAssign {
                    array: typed_array,
                    index: typed_index,
                    op: op.clone(),
                    value: typed_value,
                })
            }

            Stmt::FieldAssign { object, field, value, pos } => {
                if let Some(namespace) = self.namespace_write_target(object, field) {
                    self.errors.push(TypeError::NamespaceReadOnly { namespace, member: field.clone(), pos: pos.clone() });
                    return None;
                }
                if let Some(sname) = self.guarded_object(object) {
                    let (val, fld) = (value.clone(), field.clone());
                    let wrapped = self.wrap_guarded_write(&sname, object, field, pos, &|o| Stmt::FieldAssign {
                        object: o,
                        field: fld.clone(),
                        value: val.clone(),
                        pos: pos.clone(),
                    });
                    return self.check_stmt(&wrapped);
                }
                let typed_object = self.check_expr(object)?;
                // a struct field's declared type is what its value is checked against (`obj.tags = [1, "a"]`)
                let hint = match typed_object.ty() {
                    Ty::Named(n) => self
                        .struct_defs
                        .get(n)
                        .and_then(|fields| fields.iter().find(|p| &p.name == field))
                        .map(|p| resolve_type_expr(&p.type_ann)),
                    _ => None,
                };
                let typed_value = self.check_expr_expecting(value, hint.as_ref())?;

                if let Some((module, native)) = self.native_object_of(typed_object.ty()) {
                    let struct_name = native.name.to_string();
                    let Some(def) = native.fields.iter().find(|f| f.name == field) else {
                        self.errors.push(TypeError::NoSuchField { struct_name, field: field.clone(), pos: pos.clone() });
                        return None;
                    };
                    let Some(setter) = def.set else {
                        self.errors.push(TypeError::ReadOnlyField { struct_name, field: field.clone(), pos: pos.clone() });
                        return None;
                    };
                    let call = self.lib_call(module, setter, vec![typed_object, typed_value], pos)?;
                    return Some(TypedStmt::ExprStmt(call));
                }

                let (struct_name, field_ty) = self.resolve_field(typed_object.ty(), field, pos)?;

                if !field_ty.accepts(typed_value.ty()) {
                    self.errors.push(TypeError::StructFieldTypeMismatch {
                        struct_name,
                        field: field.clone(),
                        expected: field_ty,
                        found: typed_value.ty().clone(),
                        pos: pos.clone(),
                    });
                    return None;
                }

                Some(TypedStmt::FieldAssign { object: typed_object, field: field.clone(), value: typed_value })
            }

            Stmt::FieldCompoundAssign { object, field, op, value, pos } => {
                if let Some(namespace) = self.namespace_write_target(object, field) {
                    self.errors.push(TypeError::NamespaceReadOnly { namespace, member: field.clone(), pos: pos.clone() });
                    return None;
                }
                if let Some(sname) = self.guarded_object(object) {
                    let (val, fld, opr) = (value.clone(), field.clone(), op.clone());
                    let wrapped = self.wrap_guarded_write(&sname, object, field, pos, &|o| Stmt::FieldCompoundAssign {
                        object: o,
                        field: fld.clone(),
                        op: opr.clone(),
                        value: val.clone(),
                        pos: pos.clone(),
                    });
                    return self.check_stmt(&wrapped);
                }
                let typed_object = self.check_expr(object)?;
                let typed_value = self.check_expr(value)?;

                if let Some((module, native)) = self.native_object_of(typed_object.ty()) {
                    let struct_name = native.name.to_string();
                    let Some(def) = native.fields.iter().find(|f| f.name == field) else {
                        self.errors.push(TypeError::NoSuchField { struct_name, field: field.clone(), pos: pos.clone() });
                        return None;
                    };
                    let Some(setter) = def.set else {
                        self.errors.push(TypeError::ReadOnlyField { struct_name, field: field.clone(), pos: pos.clone() });
                        return None;
                    };
                    // the getter and the setter both take the object, but the object must be
                    // evaluated only once (it may have side effects, like `make().x += 1`).
                    // a plain name is read twice harmlessly; any other object goes through a
                    // hidden local, the same trick `when` uses for its subject.
                    let loc = SrcLoc { file: pos.file.clone(), line: pos.line };
                    let object_ty = typed_object.ty().clone();
                    let (bind, target) = if is_name_path(object) {
                        (None, typed_object)
                    } else {
                        let hidden = "$compound".to_string();
                        let bind = TypedStmt::Let {
                            name: hidden.clone(),
                            ty: object_ty.clone(),
                            value: typed_object,
                            exported: false,
                        };
                        (Some(bind), TypedExpr::Ident { name: hidden, ty: object_ty })
                    };
                    let current = self.lib_call(module, def.get, vec![target.clone()], pos)?;
                    let result_ty = self.binary_result_type(op, current.ty(), typed_value.ty(), pos)?;
                    let next = TypedExpr::Binary {
                        left: Box::new(current),
                        op: op.clone(),
                        right: Box::new(typed_value),
                        ty: result_ty,
                    };
                    let call = self.lib_call(module, setter, vec![target, next], pos)?;
                    let stmt = TypedStmt::ExprStmt(call);
                    return Some(match bind {
                        None => stmt,
                        Some(bind) => TypedStmt::Block {
                            body: TypedBlock { stmts: vec![bind, stmt], locs: vec![loc.clone(), loc], ty: Ty::Void },
                        },
                    });
                }

                let (struct_name, field_ty) = self.resolve_field(typed_object.ty(), field, pos)?;

                let result_ty = self.binary_result_type(op, &field_ty, typed_value.ty(), pos)?;
                if !field_ty.accepts(&result_ty) {
                    self.errors.push(TypeError::StructFieldTypeMismatch {
                        struct_name,
                        field: field.clone(),
                        expected: field_ty,
                        found: result_ty,
                        pos: pos.clone(),
                    });
                    return None;
                }

                Some(TypedStmt::FieldCompoundAssign {
                    object: typed_object,
                    field: field.clone(),
                    op: op.clone(),
                    value: typed_value,
                })
            }

            Stmt::Break { pos } => {
                if self.loop_depth == 0 {
                    self.errors.push(TypeError::LoopControlOutsideLoop {
                        keyword: "break".to_string(),
                        pos: pos.clone(),
                    });
                    return None;
                }
                Some(TypedStmt::Break)
            }

            Stmt::Continue { pos } => {
                if self.loop_depth == 0 {
                    self.errors.push(TypeError::LoopControlOutsideLoop {
                        keyword: "continue".to_string(),
                        pos: pos.clone(),
                    });
                    return None;
                }
                Some(TypedStmt::Continue)
            }

            // `x++;` as a statement is `x += 1`: the same typed node, so the VM and the JIT
            // keep their fused paths. the `1` is int or float, following the target's type
            Stmt::ExprStmt(Expr::IncDec { target, delta, pos, .. }) => {
                if let Expr::Field { object, field, .. } = &**target {
                    if let Some(sname) = self.guarded_object(object) {
                        let is_float = self
                            .struct_defs
                            .get(&sname)
                            .and_then(|fs| fs.iter().find(|f| &f.name == field))
                            .map_or(false, |f| resolve_type_expr(&f.type_ann) == Ty::Float);
                        let one = if is_float {
                            Expr::FloatLit(delta.abs() as f64, pos.clone())
                        } else {
                            Expr::IntLit(delta.abs(), pos.clone())
                        };
                        let op = if *delta > 0 { BinaryOp::Add } else { BinaryOp::Sub };
                        let as_compound = Stmt::FieldCompoundAssign {
                            object: (**object).clone(),
                            field: field.clone(),
                            op,
                            value: one,
                            pos: pos.clone(),
                        };
                        return self.check_stmt(&as_compound);
                    }
                }
                let typed_target = self.check_incdec_target(target, pos)?;
                let ty = typed_target.ty().clone();
                self.check_incdec_numeric(&ty, *delta, pos)?;
                let op = if *delta > 0 { p::BinaryOp::Add } else { p::BinaryOp::Sub };
                let one = if ty == Ty::Float { TypedExpr::FloatLit(1.0) } else { TypedExpr::IntLit(1) };
                match typed_target {
                    TypedExpr::Ident { name, ty } => {
                        let value = TypedExpr::Binary {
                            left: Box::new(TypedExpr::Ident { name: name.clone(), ty: ty.clone() }),
                            op,
                            right: Box::new(one),
                            ty,
                        };
                        Some(TypedStmt::Assign { name, value })
                    }
                    TypedExpr::Index { array, index, .. } => {
                        Some(TypedStmt::IndexCompoundAssign { array: *array, index: *index, op, value: one })
                    }
                    TypedExpr::Field { object, field, .. } => {
                        Some(TypedStmt::FieldCompoundAssign { object: *object, field, op, value: one })
                    }
                    _ => None,
                }
            }

            Stmt::ExprStmt(expr) => {
                let typed = self.check_expr(expr)?;
                Some(TypedStmt::ExprStmt(typed))
            }
        }
    }

    fn check_block(&mut self, block: &p::Block) -> TypedBlock {
        let mut typed_stmts = Vec::new();
        let mut locs = Vec::new();
        let mut last_loc = SrcLoc::default();
        let mut last_ty = Ty::Void;

        // `#> strict`: what follows a `return`/`break`/`continue`/`throw` in the
        // same block can never run (reported once per block). dead code, go home.
        let mut dead: Option<(&'static str, p::Pos)> = None;
        let mut reported = false;
        for stmt in &block.stmts {
            if let Some((keyword, pos)) = dead.take() {
                if self.strict && !reported {
                    reported = true;
                    self.errors.push(TypeError::UnreachableCode { keyword, pos });
                }
            }
            let loc = stmt_loc(stmt, &last_loc);
            last_loc = loc.clone();
            if let Some(typed) = self.check_stmt(stmt) {
                if let TypedStmt::ExprStmt(e) = &typed {
                    last_ty = e.ty().clone();
                }
                typed_stmts.push(typed);
                locs.push(loc);
            }
            match stmt {
                Stmt::Return { pos, .. } => dead = Some(("return", pos.clone())),
                Stmt::Break { pos } => dead = Some(("break", pos.clone())),
                Stmt::Continue { pos } => dead = Some(("continue", pos.clone())),
                Stmt::Throw { pos, .. } => dead = Some(("throw", pos.clone())),
                _ => {}
            }
        }

        TypedBlock { stmts: typed_stmts, locs, ty: last_ty }
    }

    /// a non-void function must return on EVERY possible path.
    /// (return TYPES are checked separately, at each `return`). real flow analysis: if/else needs both branches to always
    /// return. while/for never guarantee one, since the body can run
    /// zero times. try/catch needs both try and catch to return. fair.
    fn check_all_paths_return(&mut self, stmts: &[Stmt], expected: &Ty, fn_name: &str, pos: &p::Pos) {
        // inference already failed and reported CannotInferReturnType for
        // this function. skip the normal comparison to the Void
        // placeholder, it'd only add confusing, redundant noise
        if self.uninferrable_functions.contains(fn_name) {
            return;
        }

        if *expected != Ty::Void && !always_returns(stmts, &self.exhaustive_matches) {
            self.errors.push(TypeError::MissingReturn {
                function_name: fn_name.to_string(),
                expected: expected.clone(),
                pos: pos.clone(),
            });
        }
    }

    /// lightweight inference, only used by `collect_return_types_in_stmts`:
    /// reuses check_expr's logic but throws away the typed node, keeping just
    /// the resolved Ty. the caller throws away its errors
    /// (`infer_return_type` snapshots/restores `self.errors`), so it can
    /// never double-report. no echoes here.
    fn infer_expr_ty(&mut self, expr: &Expr) -> Ty {
        self.check_expr(expr).map(|e| e.ty().clone()).unwrap_or(Ty::Void)
    }

    // maps (the hash people)

    /// map keys must hash: int, string, bool or an enum. never float, array, struct.
    fn valid_map_key(&mut self, key: &Ty, pos: &p::Pos) -> bool {
        let ok = matches!(key, Ty::Int | Ty::String | Ty::Bool) || self.is_enum_ty(key);
        if !ok {
            self.errors.push(TypeError::MapKeyType { found: key.clone(), pos: pos.clone() });
        }
        ok
    }

    /// `container[index]` (read, assign or compound assign): the type of the
    /// element/value, or `None` after reporting. arrays want an `int` index,
    /// maps want their key type. don't mix them up.
    fn index_target(&mut self, container: &Ty, index: &Ty, pos: &p::Pos) -> Option<Ty> {
        match container {
            Ty::Array(inner) => {
                if index != &Ty::Int {
                    self.errors.push(TypeError::IndexTypeMismatch { found: index.clone(), pos: pos.clone() });
                    return None;
                }
                Some((**inner).clone())
            }
            Ty::Map(key, value) => {
                if !self.valid_map_key(key, pos) {
                    return None;
                }
                if **key != *index {
                    self.errors.push(TypeError::MapKeyMismatch {
                        expected: (**key).clone(),
                        found: index.clone(),
                        pos: pos.clone(),
                    });
                    return None;
                }
                Some((**value).clone())
            }
            other => {
                if index != &Ty::Int {
                    self.errors.push(TypeError::IndexTypeMismatch { found: index.clone(), pos: pos.clone() });
                } else {
                    self.errors.push(TypeError::NotIndexable { found: other.clone(), pos: pos.clone() });
                }
                None
            }
        }
    }

    /// `@array.push/contains/indexOf/insert/fill/concat`: the element (or the
    /// other array) must agree with the array's type. `fill`/`concat` know
    /// their own result type. an `array<void>` (the unannotated `[]`) accepts
    /// any element, same as `push` always did. chaos, but polite.
    /// `@task.all(tasks)`: the tasks must be an `array<task<T>>`, and the result is an `array<T>`
    fn check_task_all(&mut self, name: &str, args: &[&Ty], pos: &p::Pos) -> Option<Ty> {
        match args[0] {
            Ty::Array(elem) if matches!(**elem, Ty::Void) => Some(Ty::Array(Box::new(Ty::Void))),
            Ty::Array(elem) => match elem.as_ref() {
                Ty::Task(inner) => Some(Ty::Array(inner.clone())),
                _ => None,
            },
            _ => None,
        }
        .or_else(|| {
            self.errors.push(TypeError::ArgTypeMismatch {
                function_name: name.to_string(),
                param_index: 0,
                expected: Ty::Array(Box::new(Ty::Task(Box::new(Ty::Any)))),
                found: args[0].clone(),
                pos: pos.clone(),
            });
            None
        })
    }

    fn check_array_call(&mut self, id: BuiltinId, name: &str, args: &[&Ty], pos: &p::Pos) -> Option<Ty> {
        let mismatch = |errors: &mut Vec<TypeError>, i: usize, expected: &Ty| {
            errors.push(TypeError::ArgTypeMismatch {
                function_name: name.to_string(),
                param_index: i,
                expected: expected.clone(),
                found: args[i].clone(),
                pos: pos.clone(),
            });
        };
        let elem = match args[0] {
            Ty::Array(e) => (**e).clone(),
            _ => Ty::Void,
        };
        match id {
            BuiltinId::ArrayFill => {
                if !matches!(args[1], Ty::Int | Ty::Float | Ty::Bool | Ty::String) {
                    self.errors.push(TypeError::ArrayFillType { found: args[1].clone(), pos: pos.clone() });
                    return None;
                }
                Some(Ty::Array(Box::new(args[1].clone())))
            }
            BuiltinId::ArrayPick => Some(if matches!(elem, Ty::Void) { Ty::Any } else { elem.clone() }),
            BuiltinId::ArrayShuffle => Some(args[0].clone()),
            BuiltinId::ArrayConcat => {
                if args[0] != args[1] {
                    mismatch(&mut self.errors, 1, args[0]);
                    return None;
                }
                Some(if args[0].is_empty_array() { args[1].clone() } else { args[0].clone() })
            }
            _ => {
                let at = if id == BuiltinId::ArrayInsert { 2 } else { 1 };
                if !matches!(elem, Ty::Void) && elem != *args[at] {
                    mismatch(&mut self.errors, at, &elem);
                    return None;
                }
                Some(match id {
                    BuiltinId::ArrayContains => Ty::Bool,
                    BuiltinId::ArrayIndexOf => Ty::Int,
                    _ => Ty::Void, // push, insert
                })
            }
        }
    }

    /// the `@map.*` builtins: validates the key / fallback against the map's
    /// own types and returns the call's result type (`None` after reporting)
    fn check_map_call(&mut self, id: BuiltinId, name: &str, args: &[&Ty], pos: &p::Pos) -> Option<Ty> {
        let (key, value) = match args[0] {
            Ty::Map(k, v) => ((**k).clone(), (**v).clone()),
            _ => return None,
        };
        if !self.valid_map_key(&key, pos) {
            return None;
        }
        let mut ok = true;
        let mut want = |i: usize, expected: &Ty, errors: &mut Vec<TypeError>| {
            if args[i] != expected {
                errors.push(TypeError::ArgTypeMismatch {
                    function_name: name.to_string(),
                    param_index: i,
                    expected: expected.clone(),
                    found: args[i].clone(),
                    pos: pos.clone(),
                });
                ok = false;
            }
        };
        let result = match id {
            BuiltinId::MapHas | BuiltinId::MapRemove => {
                want(1, &key, &mut self.errors);
                Ty::Bool
            }
            BuiltinId::MapGet => {
                want(1, &key, &mut self.errors);
                want(2, &value, &mut self.errors);
                value.clone()
            }
            BuiltinId::MapKeysOf => {
                want(1, &value, &mut self.errors);
                Ty::Array(Box::new(key.clone()))
            }
            BuiltinId::MapSet => {
                want(1, &key, &mut self.errors);
                want(2, &value, &mut self.errors);
                Ty::Void
            }
            BuiltinId::MapLen => Ty::Int,
            BuiltinId::MapKeys => Ty::Array(Box::new(key.clone())),
            _ => Ty::Array(Box::new(value.clone())), // MapValues
        };
        if ok { Some(result) } else { None }
    }

    // expressions (the main event)

    /// `callee(args)` where `callee` is a function value: its arity and arg
    /// types get checked against the function type it has
    fn check_value_call(&mut self, callee: TypedExpr, args: &[Expr], pos: &p::Pos) -> Option<TypedExpr> {
        let (params, ret) = match callee.ty() {
            Ty::Function { params, ret } => (params.clone(), (**ret).clone()),
            _ => {
                self.errors.push(TypeError::NotCallable { pos: pos.clone() });
                return None;
            }
        };
        let mut typed_args = Vec::new();
        for arg in args {
            typed_args.push(self.check_expr(arg)?);
        }
        if params.len() != typed_args.len() {
            self.errors.push(TypeError::ArityMismatch {
                function_name: "this function value".to_string(),
                expected: params.len(),
                found: typed_args.len(),
                pos: pos.clone(),
            });
            return None;
        }
        for (i, (expected, arg)) in params.iter().zip(typed_args.iter()).enumerate() {
            if !ty_matches(expected, arg.ty()) {
                self.errors.push(TypeError::ArgTypeMismatch {
                    function_name: "this function value".to_string(),
                    param_index: i,
                    expected: expected.clone(),
                    found: arg.ty().clone(),
                    pos: pos.clone(),
                });
            }
        }
        Some(TypedExpr::CallValue { callee: Box::new(callee), args: typed_args, ty: ret })
    }

    /// checks `expr` where a type is expected. an array or map literal takes its element and key/value
    /// types from there, so `[1, "two"]` fits `array<int or string>` and `["a": [1, "x"]]` fits
    /// `map<string, array<int or string>>`. a `T?` expectation uses its `T`. anything else is checked
    /// as usual (`check_expr`), so the common path does not change
    fn check_expr_expecting(&mut self, expr: &Expr, expected: Option<&Ty>) -> Option<TypedExpr> {
        let target = match expected {
            Some(Ty::Optional(inner)) => inner.as_ref(),
            Some(other) => other,
            None => return self.check_expr(expr),
        };
        match (expr, target) {
            (Expr::ArrayLit(items, lit_pos), Ty::Array(elem)) => {
                let mut typed_items = Vec::new();
                for item in items {
                    let t = self.check_expr_expecting(item, Some(elem))?;
                    if !elem.accepts(t.ty()) {
                        self.errors.push(TypeError::ArrayItemNotExpected {
                            expected: (**elem).clone(),
                            found: t.ty().clone(),
                            pos: lit_pos.clone(),
                        });
                        return None;
                    }
                    typed_items.push(t);
                }
                Some(TypedExpr::ArrayLit { items: typed_items, ty: target.clone() })
            }
            (Expr::MapLit(entries, pos), Ty::Map(kt, vt)) if !entries.is_empty() => {
                if !self.valid_map_key(kt, pos) {
                    return None;
                }
                let mut typed_entries = Vec::new();
                for (k, v) in entries {
                    let tk = self.check_expr_expecting(k, Some(kt))?;
                    if !kt.accepts(tk.ty()) {
                        self.errors.push(TypeError::MapKeyMismatch {
                            expected: (**kt).clone(),
                            found: tk.ty().clone(),
                            pos: pos.clone(),
                        });
                        return None;
                    }
                    let tv = self.check_expr_expecting(v, Some(vt))?;
                    if !vt.accepts(tv.ty()) {
                        self.errors.push(TypeError::MapValueMismatch {
                            expected: (**vt).clone(),
                            found: tv.ty().clone(),
                            pos: pos.clone(),
                        });
                        return None;
                    }
                    typed_entries.push((tk, tv));
                }
                Some(TypedExpr::MapLit { entries: typed_entries, ty: target.clone() })
            }
            _ => self.check_expr(expr),
        }
    }

    /// `x in arr` / `k in map`: lowered to the builtin that answers membership
    fn check_membership(&mut self, left: &Expr, right: &Expr, pos: &p::Pos) -> Option<TypedExpr> {
        let typed_left = self.check_expr(left)?;
        let typed_right = self.check_expr(right)?;
        let (builtin, key_ty, name) = match typed_right.ty() {
            Ty::Array(elem) => (BuiltinId::ArrayContains, (**elem).clone(), "@array.contains"),
            Ty::Map(key, _) => (BuiltinId::MapHas, (**key).clone(), "@map.has"),
            // `x in s` on a string: is x inside s, as a substring (`"" in s` is true, as in Python)
            Ty::String => (BuiltinId::StrContains, Ty::String, "@str.contains"),
            other => {
                let right_ty = other.clone();
                self.errors.push(TypeError::InvalidOperandTypes {
                    op: "in".to_string(),
                    left: typed_left.ty().clone(),
                    right: right_ty,
                    pos: pos.clone(),
                });
                return None;
            }
        };
        if !key_ty.accepts(typed_left.ty()) {
            self.errors.push(TypeError::InvalidOperandTypes {
                op: "in".to_string(),
                left: typed_left.ty().clone(),
                right: typed_right.ty().clone(),
                pos: pos.clone(),
            });
            return None;
        }
        Some(TypedExpr::Call {
            callee: Box::new(TypedExpr::Ident { name: name.to_string(), ty: Ty::Void }),
            args: vec![typed_right, typed_left],
            ty: Ty::Bool,
            builtin: Some(builtin),
        })
    }

    /// `@array.first(arr)` / `@array.last(arr)`: the element, or `none` for an empty array
    fn check_first_last(&mut self, name: &str, typed_args: Vec<TypedExpr>, pos: &p::Pos) -> Option<TypedExpr> {
        if typed_args.len() != 1 {
            self.errors.push(TypeError::ArityMismatch {
                function_name: name.to_string(),
                expected: 1,
                found: typed_args.len(),
                pos: pos.clone(),
            });
            return None;
        }
        let array = typed_args.into_iter().next()?;
        let elem = match array.ty() {
            Ty::Array(inner) => (**inner).clone(),
            other => {
                let found = other.clone();
                self.arg_mismatch(name, 0, Ty::AnyArray, found, pos);
                return None;
            }
        };
        let builtin = if name == "@array.first" { BuiltinId::ArrayFirst } else { BuiltinId::ArrayLast };
        Some(TypedExpr::Call {
            callee: Box::new(TypedExpr::Ident { name: name.to_string(), ty: Ty::Void }),
            args: vec![array],
            ty: Ty::Optional(Box::new(elem)),
            builtin: Some(builtin),
        })
    }

    /// `@map.map(m, fn(k: K, v: V) -> U)` gives `map<K, U>`. `@map.filter(m, fn(k: K, v: V) -> bool)`
    /// keeps the entries where `f` gives true. both go in insertion order
    fn check_map_fn(&mut self, name: &str, typed_args: Vec<TypedExpr>, pos: &p::Pos) -> Option<TypedExpr> {
        let kind = if name == "@map.map" { MapFnKind::Map } else { MapFnKind::Filter };
        if typed_args.len() != 2 {
            self.errors.push(TypeError::ArityMismatch {
                function_name: name.to_string(),
                expected: 2,
                found: typed_args.len(),
                pos: pos.clone(),
            });
            return None;
        }
        let mut args = typed_args.into_iter();
        let map = args.next()?;
        let func = args.next()?;
        let (key_ty, val_ty) = match map.ty() {
            Ty::Map(k, v) => ((**k).clone(), (**v).clone()),
            other => {
                let found = other.clone();
                self.arg_mismatch(name, 0, Ty::AnyMap, found, pos);
                return None;
            }
        };
        let (params, ret) = match func.ty() {
            Ty::Function { params, ret } => (params.clone(), (**ret).clone()),
            other => {
                let found = other.clone();
                let expected = Ty::Function { params: vec![key_ty.clone(), val_ty.clone()], ret: Box::new(Ty::Any) };
                self.arg_mismatch(name, 1, expected, found, pos);
                return None;
            }
        };
        let entry_params = vec![key_ty.clone(), val_ty.clone()];
        let out_ty = match kind {
            MapFnKind::Map => {
                if params != entry_params {
                    let expected = Ty::Function { params: entry_params, ret: Box::new(ret.clone()) };
                    self.arg_mismatch(name, 1, expected, func.ty().clone(), pos);
                    return None;
                }
                Ty::Map(Box::new(key_ty), Box::new(ret))
            }
            MapFnKind::Filter => {
                if params != entry_params || ret != Ty::Bool {
                    let expected = Ty::Function { params: entry_params, ret: Box::new(Ty::Bool) };
                    self.arg_mismatch(name, 1, expected, func.ty().clone(), pos);
                    return None;
                }
                Ty::Map(Box::new(key_ty), Box::new(val_ty))
            }
        };
        Some(TypedExpr::MapFn { kind, map: Box::new(map), func: Box::new(func), ty: out_ty })
    }

    /// `@array.map(arr, f)` gives `U[]` when `f: (T) -> U`. `@array.filter(arr, f)`
    /// keeps the elements where `f` gives true. `@array.reduce(arr, f, init)` folds
    /// with `f: (A, T) -> A`, starting from `init`
    fn check_array_fn(&mut self, name: &str, typed_args: Vec<TypedExpr>, pos: &p::Pos) -> Option<TypedExpr> {
        let (kind, arity) = match name {
            "@array.map" => (ArrayFnKind::Map, 2),
            "@array.filter" => (ArrayFnKind::Filter, 2),
            _ => (ArrayFnKind::Reduce, 3),
        };
        if typed_args.len() != arity {
            self.errors.push(TypeError::ArityMismatch {
                function_name: name.to_string(),
                expected: arity,
                found: typed_args.len(),
                pos: pos.clone(),
            });
            return None;
        }
        let mut args = typed_args.into_iter();
        let array = args.next()?;
        let func = args.next()?;
        let init = if kind == ArrayFnKind::Reduce { args.next() } else { None };

        let elem = match array.ty() {
            Ty::Array(inner) => (**inner).clone(),
            other => {
                let found = other.clone();
                self.arg_mismatch(name, 0, Ty::AnyArray, found, pos);
                return None;
            }
        };
        let (params, ret) = match func.ty() {
            Ty::Function { params, ret } => (params.clone(), (**ret).clone()),
            other => {
                let found = other.clone();
                let expected = Ty::Function { params: vec![elem.clone()], ret: Box::new(Ty::Any) };
                self.arg_mismatch(name, 1, expected, found, pos);
                return None;
            }
        };

        let (ty, init) = match kind {
            ArrayFnKind::Map => {
                if params != vec![elem.clone()] {
                    let expected = Ty::Function { params: vec![elem.clone()], ret: Box::new(ret.clone()) };
                    self.arg_mismatch(name, 1, expected, func.ty().clone(), pos);
                    return None;
                }
                (Ty::Array(Box::new(ret)), None)
            }
            ArrayFnKind::Filter => {
                if params != vec![elem.clone()] || ret != Ty::Bool {
                    let expected = Ty::Function { params: vec![elem.clone()], ret: Box::new(Ty::Bool) };
                    self.arg_mismatch(name, 1, expected, func.ty().clone(), pos);
                    return None;
                }
                (Ty::Array(Box::new(elem)), None)
            }
            ArrayFnKind::Reduce => {
                let init = init?;
                let acc = init.ty().clone();
                if params != vec![acc.clone(), elem.clone()] || ret != acc {
                    let expected = Ty::Function { params: vec![acc.clone(), elem.clone()], ret: Box::new(acc.clone()) };
                    self.arg_mismatch(name, 1, expected, func.ty().clone(), pos);
                    return None;
                }
                (acc, Some(Box::new(init)))
            }
        };
        Some(TypedExpr::ArrayFn { kind, array: Box::new(array), func: Box::new(func), init, ty })
    }

    fn arg_mismatch(&mut self, name: &str, param_index: usize, expected: Ty, found: Ty, pos: &p::Pos) {
        self.errors.push(TypeError::ArgTypeMismatch {
            function_name: name.to_string(),
            param_index,
            expected,
            found,
            pos: pos.clone(),
        });
    }

    /// the typed place a `++`/`--` works on: a variable, an array element or a
    /// struct field. same rules as the plain assignment to that place
    fn check_incdec_target(&mut self, target: &Expr, pos: &p::Pos) -> Option<TypedExpr> {
        match target {
            Expr::Ident(name, npos) => {
                let found = self.lookup_var(name).map(|info| (info.mutable, info.ty.clone()));
                let (mutable, ty) = match found {
                    Some(found) => found,
                    None => {
                        self.errors.push(TypeError::UndefinedVariable { name: name.clone(), pos: npos.clone() });
                        return None;
                    }
                };
                if self.assigns_capture(name) {
                    self.errors.push(TypeError::CaptureAssign { name: name.clone(), pos: npos.clone() });
                    return None;
                }
                if !mutable {
                    self.errors.push(TypeError::AssignToConst { name: name.clone(), pos: npos.clone() });
                    return None;
                }
                self.mark_assigned(name);
                Some(TypedExpr::Ident { name: name.clone(), ty })
            }
            Expr::Index { array, index, pos: ipos } => {
                let typed_array = self.check_expr(array)?;
                let typed_index = self.check_expr(index)?;
                let elem_ty = self.index_target(typed_array.ty(), typed_index.ty(), ipos)?;
                Some(TypedExpr::Index { array: Box::new(typed_array), index: Box::new(typed_index), ty: elem_ty })
            }
            Expr::Field { object, field, pos: fpos } => {
                let typed_object = self.check_expr(object)?;
                if self.native_object_of(typed_object.ty()).is_some() {
                    self.errors.push(TypeError::IncDecTarget { pos: pos.clone() });
                    return None;
                }
                let (_, field_ty) = self.resolve_field(typed_object.ty(), field, fpos)?;
                Some(TypedExpr::Field { object: Box::new(typed_object), field: field.clone(), ty: field_ty })
            }
            _ => {
                self.errors.push(TypeError::IncDecTarget { pos: pos.clone() });
                None
            }
        }
    }

    /// `++`/`--` only count int and float
    fn check_incdec_numeric(&mut self, ty: &Ty, delta: i64, pos: &p::Pos) -> Option<()> {
        if matches!(ty, Ty::Int | Ty::Float) {
            return Some(());
        }
        let op_text = if delta > 0 { "++" } else { "--" };
        self.errors.push(TypeError::InvalidOperandTypes {
            op: op_text.to_string(),
            left: ty.clone(),
            right: ty.clone(),
            pos: pos.clone(),
        });
        None
    }

    /// `int::max` and friends: the property is a constant, so it becomes a literal
    /// right here (the compiler never sees a property). the list is the whole API.
    fn check_type_prop(&mut self, ty: &str, name: &str, pos: &p::Pos) -> Option<TypedExpr> {
        let value = match (ty, name) {
            // `int::methods`: the native methods of the type, then the user ones in `add` order.
            // a snapshot: an array of names, the same every time it is read
            ("int" | "float" | "bool" | "string", "methods") => {
                let vty = match ty {
                    "int" => Ty::Int,
                    "float" => Ty::Float,
                    "bool" => Ty::Bool,
                    _ => Ty::String,
                };
                let mut names = native_method_names(&vty);
                let prefix = format!("{}.", ty);
                let mut user: Vec<(usize, String)> = Vec::new();
                // only the ones this file can call: its own, and the exported ones of the others
                for (k, ms) in &self.value_methods {
                    if let Some(member) = k.strip_prefix(prefix.as_str()) {
                        user.extend(
                            ms.iter()
                                .filter(|m| m.exported || m.file == pos.file)
                                .map(|m| (m.seq, member.to_string())),
                        );
                    }
                }
                user.sort();
                names.extend(user.into_iter().map(|(_, n)| n));
                let items = names.into_iter().map(TypedExpr::StringLit).collect();
                TypedExpr::ArrayLit { items, ty: Ty::Array(Box::new(Ty::String)) }
            }
            // `array::methods`: the native array functions, sorted. a snapshot, like the others.
            ("array", "methods") => {
                let items = native_array_method_names().into_iter().map(TypedExpr::StringLit).collect();
                TypedExpr::ArrayLit { items, ty: Ty::Array(Box::new(Ty::String)) }
            }
            ("int", "bits") => TypedExpr::IntLit(64),
            // the characters of text: plain strings, so `in` and `for` work on them directly
            ("string", "newline") => TypedExpr::StringLit("\n".to_string()),
            ("string", "tab") => TypedExpr::StringLit("\t".to_string()),
            ("string", "empty") => TypedExpr::StringLit(String::new()),
            ("string", "digits") => TypedExpr::StringLit("0123456789".to_string()),
            ("string", "lower") => TypedExpr::StringLit("abcdefghijklmnopqrstuvwxyz".to_string()),
            ("string", "upper") => TypedExpr::StringLit("ABCDEFGHIJKLMNOPQRSTUVWXYZ".to_string()),
            ("int", "max") => TypedExpr::IntLit(i64::MAX),
            ("int", "min") => TypedExpr::IntLit(i64::MIN),
            ("float", "max") => TypedExpr::FloatLit(f64::MAX),
            ("float", "min") => TypedExpr::FloatLit(f64::MIN), // most negative, like int::min
            ("float", "epsilon") => TypedExpr::FloatLit(f64::EPSILON),
            ("float", "inf") => TypedExpr::FloatLit(f64::INFINITY),
            ("float", "nan") => TypedExpr::FloatLit(f64::NAN),
            _ => {
                self.errors.push(TypeError::UnknownTypeProperty {
                    ty: ty.to_string(),
                    name: name.to_string(),
                    pos: pos.clone(),
                });
                return None;
            }
        };
        Some(value)
    }

    fn check_expr(&mut self, expr: &Expr) -> Option<TypedExpr> {
        match expr {
            Expr::TypeProp { ty, name, pos } => self.check_type_prop(ty, name, pos),
            Expr::IntLit(n, _) => Some(TypedExpr::IntLit(*n)),
            Expr::FloatLit(f, _) => Some(TypedExpr::FloatLit(*f)),
            Expr::StringLit(s, _) => Some(TypedExpr::StringLit(s.clone())),
            Expr::BoolLit(b, _) => Some(TypedExpr::BoolLit(*b)),
            Expr::NoneLit(_) => Some(TypedExpr::NoneLit { ty: Ty::NoneLit }),

            Expr::ArrayLit(items, lit_pos) => {
                let mut typed_items = Vec::new();
                let mut elem_ty = Ty::Void;
                for (i, item) in items.iter().enumerate() {
                    let t = self.check_expr(item)?;
                    if i == 0 {
                        elem_ty = t.ty().clone();
                    } else if let Some(unified) = unify_optional(&elem_ty, t.ty()) {
                        elem_ty = unified;
                    } else if *t.ty() != elem_ty {
                        self.errors.push(TypeError::ArrayItemMismatch {
                            expected: elem_ty.clone(),
                            found: t.ty().clone(),
                            pos: lit_pos.clone(),
                        });
                        return None;
                    } else if elem_ty.is_empty_array() || elem_ty.is_empty_map() {
                        // `[[], [1]]`: keep the item type that actually says something. the empty one is lying.
                        elem_ty = t.ty().clone();
                    }
                    typed_items.push(t);
                }
                Some(TypedExpr::ArrayLit { items: typed_items, ty: Ty::Array(Box::new(elem_ty)) })
            }

            Expr::MapLit(entries, pos) => {
                if entries.is_empty() {
                    let ty = Ty::Map(Box::new(Ty::Void), Box::new(Ty::Void));
                    return Some(TypedExpr::MapLit { entries: Vec::new(), ty });
                }
                let mut typed_entries = Vec::new();
                let mut types: Option<(Ty, Ty)> = None;
                for (k, v) in entries {
                    let tk = self.check_expr(k)?;
                    let tv = self.check_expr(v)?;
                    match &types {
                        None => {
                            if !self.valid_map_key(tk.ty(), pos) {
                                return None;
                            }
                            types = Some((tk.ty().clone(), tv.ty().clone()));
                        }
                        Some((kt, vt)) => {
                            if tk.ty() != kt {
                                self.errors.push(TypeError::MapKeyMismatch {
                                    expected: kt.clone(),
                                    found: tk.ty().clone(),
                                    pos: pos.clone(),
                                });
                                return None;
                            }
                            if !vt.accepts(tv.ty()) {
                                self.errors.push(TypeError::MapValueMismatch {
                                    expected: vt.clone(),
                                    found: tv.ty().clone(),
                                    pos: pos.clone(),
                                });
                                return None;
                            }
                        }
                    }
                    typed_entries.push((tk, tv));
                }
                let (kt, vt) = types.expect("a non-empty literal sets the types");
                Some(TypedExpr::MapLit { entries: typed_entries, ty: Ty::Map(Box::new(kt), Box::new(vt)) })
            }

            Expr::Ident(name, pos) => {
                if let Some(full) = self.namespace_member(name) {
                    return self.check_expr(&Expr::Ident(full, pos.clone()));
                }
                if let Some(ty) = self.resolve_name(name) {
                    Some(TypedExpr::Ident { name: name.clone(), ty })
                } else if let Some(sig) = self.functions.get(name) {
                    // a named function used as a value (`let f = dobro`). functions are people too.
                    let params: Vec<Ty> = sig.params.iter().map(|p| resolve_type_expr(&p.type_ann)).collect();
                    let ty = Ty::Function { params, ret: Box::new(sig.return_type.clone()) };
                    Some(TypedExpr::Ident { name: name.clone(), ty })
                } else if self.enum_defs.contains_key(name) {
                    self.errors.push(TypeError::EnumAsValue { name: name.clone(), pos: pos.clone() });
                    None
                } else {
                    if spg_lexer::is_install_namespace(name.as_str()) {
                        // a bare `http` / `ps` (no `.member`): say what it is instead of "undefined"
                        self.errors.push(TypeError::NeedsLoad { lib: name.to_string(), pos: pos.clone() });
                    } else {
                        self.errors.push(TypeError::UndefinedVariable { name: name.clone(), pos: pos.clone() });
                    }
                    None
                }
            }

            Expr::Binary { left, op: BinaryOp::In, right, pos } => self.check_membership(left, right, pos),

            Expr::Binary { left, op, right, pos } => {
                let mut typed_left = self.check_expr(left)?;
                // `a != none && a.x > 0`: the right side sees what the left side proved
                let proven = match op {
                    BinaryOp::And => narrowing(left).0,
                    BinaryOp::Or => narrowing(left).1,
                    _ => Vec::new(),
                };
                let type_proven = match op {
                    BinaryOp::And => type_facts(left).0,
                    BinaryOp::Or => type_facts(left).1,
                    _ => Vec::new(),
                };
                let checked_right = if proven.is_empty() && type_proven.is_empty() {
                    self.check_expr(right)
                } else {
                    self.push_scope();
                    self.narrow(&proven);
                    self.narrow_types(&type_proven);
                    let r = self.check_expr(right);
                    self.pop_scope();
                    r
                };
                let mut typed_right = checked_right?;
                if matches!(op, BinaryOp::Coalesce) {
                    return self.coalesce(typed_left, typed_right, pos);
                }
                let result_ty = self.binary_result_type(op, typed_left.ty(), typed_right.ty(), pos)?;
                // `==` / `!=` on two values of the same enum: the VM compares them
                // with the plain int opcodes, so both operands read as ints. cheap trick.
                if matches!(op, BinaryOp::Eq | BinaryOp::NotEq) && self.is_enum_ty(typed_left.ty()) {
                    typed_left = TypedExpr::AsInt(Box::new(typed_left));
                    typed_right = TypedExpr::AsInt(Box::new(typed_right));
                }
                Some(TypedExpr::Binary {
                    left: Box::new(typed_left),
                    op: op.clone(),
                    right: Box::new(typed_right),
                    ty: result_ty,
                })
            }

            Expr::IncDec { target, delta, prefix, pos } => {
                if let Expr::Field { object, .. } = &**target {
                    if let Some(struct_name) = self.guarded_object(object) {
                        self.errors.push(TypeError::GuardedIncDecValue { struct_name, pos: pos.clone() });
                        return None;
                    }
                }
                let typed_target = self.check_incdec_target(target, pos)?;
                let ty = typed_target.ty().clone();
                self.check_incdec_numeric(&ty, *delta, pos)?;
                Some(TypedExpr::IncDec { target: Box::new(typed_target), delta: *delta, prefix: *prefix, ty })
            }

            Expr::Unary { op, expr, pos } => {
                let typed_expr = self.check_expr(expr)?;
                let ty = match (op, typed_expr.ty()) {
                    (UnaryOp::Neg, Ty::Int) => Ty::Int,
                    (UnaryOp::Neg, Ty::Float) => Ty::Float,
                    (UnaryOp::Not, Ty::Bool) => Ty::Bool,
                    (_, other) => {
                        self.errors.push(TypeError::InvalidOperandTypes {
                            op: format!("{:?}", op),
                            left: other.clone(),
                            right: other.clone(),
                            pos: pos.clone(),
                        });
                        return None;
                    }
                };
                Some(TypedExpr::Unary { op: op.clone(), expr: Box::new(typed_expr), ty })
            }

            Expr::Call { callee, args, pos } => {
                // a callee that's neither a name nor a method (`makeAdder(2)(3)`,
                // a lambda called in place, `(f)(x)`) is an expression to evaluate
                if !matches!(callee.as_ref(), Expr::Ident(..) | Expr::Field { .. }) {
                    let typed_callee = self.check_expr(callee)?;
                    return self.check_value_call(typed_callee, args, pos);
                }
                let name = match callee.as_ref() {
                    Expr::Ident(n, _) => n.clone(),
                    // `math.sqrt(x)`: the pre-`@` namespace spelling. only when
                    // `math` isn't a variable the user declared (a struct
                    // field holding... nothing callable, but `math.x` is a
                    // legal field read, so the call is still an error below. sigh.
                    Expr::Field { object, field, .. }
                        if matches!(object.as_ref(), Expr::Ident(ns, _)
                            if matches!(ns.as_str(), "math" | "time" | "list" | "transform")
                                && self.lookup_var(ns).is_none()) =>
                    {
                        let ns = match object.as_ref() {
                            Expr::Ident(ns, _) => ns.clone(),
                            _ => unreachable!(),
                        };
                        self.errors.push(TypeError::OldNamespace {
                            name: format!("{}.{}", ns, field),
                            replacement: crate::builtins::renamed_namespace_call(&ns, field),
                            pos: pos.clone(),
                        });
                        return None;
                    }
                    // `ps.run(...)` with no `load #ps` (with it, the lexer already made
                    // it a namespace call). tell them what's missing instead of "undefined ps"
                    Expr::Field { object, .. }
                        if matches!(object.as_ref(), Expr::Ident(ns, _)
                            if spg_lexer::is_install_namespace(ns) && self.lookup_var(ns).is_none()) =>
                    {
                        let lib = match object.as_ref() {
                            Expr::Ident(ns, _) => ns.clone(),
                            _ => unreachable!(),
                        };
                        self.errors.push(TypeError::NeedsLoad { lib, pos: pos.clone() });
                        return None;
                    }
                    // `obj.method(args)`: the only way a struct member gets called
                    Expr::Field { object, field, .. } => {
                        return self.check_method_call(object, field, args, pos);
                    }
                    _ => {
                        self.errors.push(TypeError::NotCallable { pos: pos.clone() });
                        return None;
                    }
                };

                // a sibling function inside `namespace som`: `add(a, b)` means `som.add(a, b)`
                let name = self.namespace_member(&name).unwrap_or(name);

                // a variable holding a function value (a local, a param, a
                // global): called through its value. a local shadows a builtin or
                // a function of the same name, like any other binding would
                if let Some(ty) = self.resolve_name(&name) {
                    let callee = TypedExpr::Ident { name: name.clone(), ty };
                    return self.check_value_call(callee, args, pos);
                }

                // typecheck args once, up front. overload resolution
                // below matches these typed_args against every
                // candidate signature, no re-checking expressions. efficient.
                let user_params: Vec<Ty> = if builtin_signatures().contains_key(&name) {
                    Vec::new()
                } else {
                    self.functions
                        .get(&name)
                        .map(|sig| sig.params.iter().map(|p| resolve_type_expr(&p.type_ann)).collect())
                        .unwrap_or_default()
                };
                let mut typed_args = Vec::new();
                for (i, arg) in args.iter().enumerate() {
                    typed_args.push(self.check_expr_expecting(arg, user_params.get(i))?);
                }

                // the array loops: their result type depends on the function argument,
                // so the signature table can't express them (see `check_array_fn`)
                if matches!(name.as_str(), "@array.map" | "@array.filter" | "@array.reduce") {
                    return self.check_array_fn(&name, typed_args, pos);
                }
                if matches!(name.as_str(), "@array.first" | "@array.last") {
                    return self.check_first_last(&name, typed_args, pos);
                }
                if matches!(name.as_str(), "@map.map" | "@map.filter") {
                    return self.check_map_fn(&name, typed_args, pos);
                }

                // builtins have their own signature table (see `check_builtin_call`)
                if builtin_signatures().contains_key(&name) {
                    return self.check_builtin_call(name, typed_args, pos);
                }

                let sig = match self.functions.get(&name).cloned() {
                    Some(sig) => sig,
                    None => {
                        match crate::builtins::migrated_builtin(&name) {
                            Some(replacement) => self.errors.push(TypeError::RemovedBuiltin {
                                name: name.clone(),
                                replacement: replacement.to_string(),
                                pos: pos.clone(),
                            }),
                            None => self.errors.push(TypeError::UndefinedFunction { name: name.clone(), pos: pos.clone() }),
                        }
                        return None;
                    }
                };
                let expected_params: Vec<Ty> = sig.params.iter().map(|p| resolve_type_expr(&p.type_ann)).collect();
                let expected_return = sig.return_type.clone();

                let required = sig.params.iter().filter(|p| p.default.is_none()).count();
                if typed_args.len() < required || typed_args.len() > expected_params.len() {
                    self.errors.push(TypeError::ArityMismatch {
                        function_name: name.clone(),
                        expected: expected_params.len(),
                        found: typed_args.len(),
                        pos: pos.clone(),
                    });
                    return None;
                }

                for (i, (typed_arg, expected_ty)) in typed_args.iter().zip(expected_params.iter()).enumerate() {
                    if !ty_matches(expected_ty, typed_arg.ty()) {
                        self.errors.push(TypeError::ArgTypeMismatch {
                            function_name: name.clone(),
                            param_index: i,
                            expected: expected_ty.clone(),
                            found: typed_arg.ty().clone(),
                            pos: pos.clone(),
                        });
                    }
                }

                // args the caller left out take the param's default value,
                // checked against the param's type, like any written arg
                let mut typed_args = typed_args;
                for i in typed_args.len()..sig.params.len() {
                    let Some(default) = sig.params[i].default.as_ref() else { break };
                    let Some(typed_default) = self.check_expr(default) else { return None };
                    if !ty_matches(&expected_params[i], typed_default.ty()) {
                        self.errors.push(TypeError::ArgTypeMismatch {
                            function_name: name.clone(),
                            param_index: i,
                            expected: expected_params[i].clone(),
                            found: typed_default.ty().clone(),
                            pos: pos.clone(),
                        });
                    }
                    typed_args.push(typed_default);
                }

                Some(TypedExpr::Call {
                    callee: Box::new(TypedExpr::Ident { name, ty: expected_return.clone() }),
                    args: typed_args,
                    ty: expected_return,
                    builtin: None,
                })
            }

            Expr::Index { array, index, pos } => {
                let typed_array = self.check_expr(array)?;
                let typed_index = self.check_expr(index)?;

                let elem_ty = self.index_target(typed_array.ty(), typed_index.ty(), pos)?;

                Some(TypedExpr::Index { array: Box::new(typed_array), index: Box::new(typed_index), ty: elem_ty })
            }

            Expr::StructLit { name, fields, pos } => {
                if self.guarded_structs.contains(name) {
                    let key = (pos.line, pos.col);
                    if self.guard_literals.contains(&key) {
                        self.guard_literals.remove(&key); // the literal inside the check call
                    } else {
                        self.guard_literals.insert(key);
                        let call = Expr::Call {
                            callee: Box::new(Expr::Ident(format!("{}.__check", name), pos.clone())),
                            args: vec![expr.clone()],
                            pos: pos.clone(),
                        };
                        return self.check_expr(&call);
                    }
                }
                let struct_fields = match self.struct_defs.get(name) {
                    Some(f) => f.clone(),
                    None => {
                        self.errors.push(TypeError::UndefinedStruct { name: name.clone(), pos: pos.clone() });
                        return None;
                    }
                };

                // typecheck every given field value up front (by the
                // name the user wrote). same "check everything, then
                // decide" shape check_call uses for builtin overloads.
                // lets us report every problem in one
                // literal instead of bailing at the first. complaints, all at once.
                let mut given: HashMap<String, TypedExpr> = HashMap::new();
                for (field_name, value_expr) in fields {
                    let hint = struct_fields.iter().find(|p| &p.name == field_name).map(|p| resolve_type_expr(&p.type_ann));
                    let typed_value = self.check_expr_expecting(value_expr, hint.as_ref())?;
                    if !struct_fields.iter().any(|p| &p.name == field_name) {
                        self.errors.push(TypeError::UnknownField {
                            struct_name: name.clone(),
                            field: field_name.clone(),
                            pos: pos.clone(),
                        });
                        continue;
                    }
                    given.insert(field_name.clone(), typed_value);
                }

                let missing: Vec<String> =
                    struct_fields.iter().map(|p| p.name.clone()).filter(|n| !given.contains_key(n)).collect();
                if !missing.is_empty() {
                    self.errors.push(TypeError::MissingFields { struct_name: name.clone(), missing, pos: pos.clone() });
                    return None;
                }

                // reorder into declaration order and check each value's
                // type against its field's declared type. the compiler
                // relies on this order to emit MakeStruct by position
                let mut ordered_values = Vec::with_capacity(struct_fields.len());
                for field in &struct_fields {
                    let expected_ty = resolve_type_expr(&field.type_ann);
                    // `given` is guaranteed to have every struct_fields
                    // name at this point (the `missing` check above
                    // already returned early otherwise. trust the flow.
                    let typed_value = given.remove(&field.name).expect("checked above: field present");
                    if !expected_ty.accepts(typed_value.ty()) {
                        self.errors.push(TypeError::StructFieldTypeMismatch {
                            struct_name: name.clone(),
                            field: field.name.clone(),
                            expected: expected_ty,
                            found: typed_value.ty().clone(),
                            pos: pos.clone(),
                        });
                    }
                    ordered_values.push(typed_value);
                }

                Some(TypedExpr::StructLit { name: name.clone(), fields: ordered_values, ty: Ty::Named(name.clone()) })
            }

            Expr::Field { object, field, pos } => {
                // `som.calls` reads a namespace static: it is the variable `som.calls`
                if let Expr::Ident(ns, _) = object.as_ref() {
                    let full = format!("{}.{}", ns, field);
                    if self.lookup_var(ns).is_none() && self.is_type_name(ns) {
                        // a private member of a namespace declared in another file, read as a value
                        if let Some((namespace, member)) = self.namespace_private(&full, pos) {
                            self.errors.push(TypeError::NamespaceMemberPrivate { namespace, member, pos: pos.clone() });
                            return None;
                        }
                        // a static, or a namespace function used as a value (`let f = som.add`)
                        if self.lookup_var(&full).is_some() || self.functions.contains_key(&full) {
                            return self.check_expr(&Expr::Ident(full, pos.clone()));
                        }
                    }
                }
                // `Color.Green`: the enum's name (not a variable) before the dot
                if let Expr::Ident(n, _) = object.as_ref() {
                    if let Some(variants) = self.enum_named(n) {
                        let variants = variants.clone();
                        return match variants.iter().position(|v| v == field) {
                            Some(index) => {
                                Some(TypedExpr::EnumLit { name: n.clone(), index, ty: Ty::Named(n.clone()) })
                            }
                            None => {
                                self.errors.push(TypeError::NoSuchVariant {
                                    enum_name: n.clone(),
                                    variant: field.clone(),
                                    variants,
                                    pos: pos.clone(),
                                });
                                None
                            }
                        };
                    }
                }
                let typed_object = self.check_expr(object)?;
                // `c.value`: the value an enum variant was declared with
                if field == "value" && self.is_enum_ty(typed_object.ty()) {
                    if let Ty::Named(enum_name) = typed_object.ty().clone() {
                        let ty = self.enum_value_tys.get(&enum_name).cloned().unwrap_or(Ty::Int);
                        return Some(TypedExpr::EnumValue { operand: Box::new(typed_object), enum_name, ty });
                    }
                }
                if let Some((module, object)) = self.native_object_of(typed_object.ty()) {
                    return self.native_field_read(module, object, typed_object, field, pos);
                }
                let (_struct_name, field_ty) = self.resolve_field(typed_object.ty(), field, pos)?;
                Some(TypedExpr::Field { object: Box::new(typed_object), field: field.clone(), ty: field_ty })
            }

            Expr::Lambda { params, return_type, body, pos } => {
                let ret = return_type.as_ref().map(resolve_type_expr).unwrap_or(Ty::Void);
                let base = self.scopes.len();
                self.lambdas.push(LambdaFrame { base, captures: Vec::new() });
                self.push_scope();
                for p in params {
                    self.declare(&p.name, resolve_type_expr(&p.type_ann), true, pos);
                }

                let saved_loop_depth = self.loop_depth;
                self.loop_depth = 0;
                // `-> task<T>`: the body returns a plain T, the task wrapping happens at call time
                let body_return = match &ret {
                    Ty::Task(inner) => inner.as_ref().clone(),
                    other => other.clone(),
                };
                let saved_return = self.current_return.replace(body_return.clone());
                let typed_body = self.check_block(body);
                self.current_return = saved_return;
                self.check_all_paths_return(&body.stmts, &body_return, "<lambda>", pos);
                self.loop_depth = saved_loop_depth;

                self.pop_scope();
                let frame = self.lambdas.pop().expect("the lambda frame pushed above");

                let ty = Ty::Function {
                    params: params.iter().map(|p| resolve_type_expr(&p.type_ann)).collect(),
                    ret: Box::new(ret.clone()),
                };
                Some(TypedExpr::Lambda {
                    params: params.clone(),
                    return_type: ret,
                    body: typed_body,
                    captures: frame.captures,
                    ty,
                })
            }

            Expr::IfExpr { cond, then_branch, else_branch, pos } => {
                let typed_cond = self.check_expr(cond)?;
                if typed_cond.ty() != &Ty::Bool {
                    self.errors.push(TypeError::NonBoolCondition { found: typed_cond.ty().clone(), pos: pos.clone() });
                }

                let (when_true, when_false) = narrowing(cond);
                let (type_true, type_false) = type_facts(cond);

                self.push_scope();
                self.narrow(&when_true);
                self.narrow_types(&type_true);
                let typed_then = self.check_block(then_branch);
                self.pop_scope();

                self.push_scope();
                self.narrow(&when_false);
                self.narrow_types(&type_false);
                let typed_else = self.check_block(else_branch);
                self.pop_scope();

                if typed_then.ty != typed_else.ty {
                    self.errors.push(TypeError::IfExprBranchMismatch {
                        then_ty: typed_then.ty.clone(),
                        else_ty: typed_else.ty.clone(),
                        pos: pos.clone(),
                    });
                    return None;
                }

                let ty = typed_then.ty.clone();
                Some(TypedExpr::IfExpr {
                    cond: Box::new(typed_cond),
                    then_branch: Box::new(typed_then),
                    else_branch: Box::new(typed_else),
                    ty,
                })
            }
        }
    }

    /// the native object `ty` is, if it is one of the installed libraries' objects
    fn native_object_of(&self, ty: &Ty) -> Option<(&'static str, &'static spg_libs::NativeObject)> {
        match ty {
            Ty::Named(name) => self.native_objects.get(name).copied(),
            _ => None,
        }
    }

    /// `module.function(args)` as a checked builtin call: the same path a plain library call takes
    fn lib_call(&mut self, module: &str, function: &str, args: Vec<TypedExpr>, pos: &p::Pos) -> Option<TypedExpr> {
        self.check_builtin_call(format!("@{}.{}", module, function), args, pos)
    }

    /// `obj.field` on a native object: the getter's call. `obj` is typed already
    fn native_field_read(
        &mut self,
        module: &str,
        object: &spg_libs::NativeObject,
        typed_object: TypedExpr,
        field: &str,
        pos: &p::Pos,
    ) -> Option<TypedExpr> {
        let Some(def) = object.fields.iter().find(|f| f.name == field) else {
            self.errors.push(TypeError::NoSuchField {
                struct_name: object.name.to_string(),
                field: field.to_string(),
                pos: pos.clone(),
            });
            return None;
        };
        self.lib_call(module, def.get, vec![typed_object], pos)
    }

    /// resolves `ty.field` to `(struct_name, field_type)`, or pushes the
    /// right error and returns `None`. shared by `Expr::Field` (read),
    /// `Stmt::FieldAssign`, and `Stmt::FieldCompoundAssign` (write), so
    /// all three reject the same non-struct types and unknown fields
    /// the same way. consistent, like a good tax form.
    ///
    /// `Ty::Named` isn't only "a declared struct". `catch (e)`
    /// without an explicit type gives `Ty::Named("Error")` with no
    /// matching `struct_defs` entry (spg has no user-facing way to
    /// declare Error's shape yet). it's treated like "no
    /// fields" here instead of panicking: `catch (e) { print(e.msg) }`
    /// is a real program you can actually run. not a checker bug. (phew.)
    fn resolve_field(&mut self, ty: &Ty, field: &str, pos: &p::Pos) -> Option<(String, Ty)> {
        if self.is_enum_ty(ty) {
            self.errors.push(TypeError::NotAStruct { found: ty.clone(), pos: pos.clone() });
            return None;
        }
        let struct_name = match ty {
            Ty::Named(n) => n.clone(),
            other => {
                self.errors.push(TypeError::NotAStruct { found: other.clone(), pos: pos.clone() });
                return None;
            }
        };

        let struct_fields = match self.struct_defs.get(&struct_name) {
            Some(f) => f,
            None => {
                self.errors.push(TypeError::NoSuchField {
                    struct_name: struct_name.clone(),
                    field: field.to_string(),
                    pos: pos.clone(),
                });
                return None;
            }
        };

        if !struct_fields.iter().any(|p| p.name == field)
            && self.functions.contains_key(&format!("{}.{}", struct_name, field))
        {
            self.errors.push(TypeError::MethodNotCalled {
                struct_name,
                method: field.to_string(),
                pos: pos.clone(),
            });
            return None;
        }

        match struct_fields.iter().find(|p| p.name == field) {
            Some(p) => {
                let field_ty = resolve_type_expr(&p.type_ann);
                Some((struct_name, field_ty))
            }
            None => {
                self.errors.push(TypeError::NoSuchField { struct_name, field: field.to_string(), pos: pos.clone() });
                None
            }
        }
    }

    fn binary_result_type(&mut self, op: &BinaryOp, left: &Ty, right: &Ty, pos: &p::Pos) -> Option<Ty> {
        use BinaryOp::*;
        let ty = match (op, left, right) {
            (Add | Sub | Mul | Div | Mod | Pow, Ty::Int, Ty::Int) => Ty::Int,
            (Add | Sub | Mul | Div | Mod | Pow, Ty::Float, Ty::Float) => Ty::Float,
            (Add, Ty::String, Ty::String) => Ty::String, // string concatenation

            // `x == none`, `x != none`, and an optional against a plain value (`x == 3`)
            (Eq | NotEq, Ty::Optional(i), other) | (Eq | NotEq, other, Ty::Optional(i))
                if matches!(other, Ty::NoneLit) || **i == *other || matches!(other, Ty::Optional(j) if j == i) =>
            {
                Ty::Bool
            }
            (Eq | NotEq, Ty::NoneLit, Ty::NoneLit) => Ty::Bool,
            // a union against one of its members (`x == 30` where x is `string or int`)
            (Eq | NotEq, Ty::Union(members), other) | (Eq | NotEq, other, Ty::Union(members))
                if !matches!(other, Ty::Union(_)) && members.iter().any(|m| m.accepts(other)) =>
            {
                Ty::Bool
            }
            (Eq | NotEq, a, b) if a == b => Ty::Bool,
            (Lt | Gt | LtEq | GtEq, Ty::Int, Ty::Int) => Ty::Bool,
            (Lt | Gt | LtEq | GtEq, Ty::Float, Ty::Float) => Ty::Bool,

            (And | Or, Ty::Bool, Ty::Bool) => Ty::Bool,

            _ => {
                self.errors.push(TypeError::InvalidOperandTypes {
                    op: format!("{:?}", op),
                    left: left.clone(),
                    right: right.clone(),
                    pos: pos.clone(),
                });
                return None;
            }
        };
        Some(ty)
    }
}

/// figures out a builtin's return type when it's a wildcard
/// (`Ty::AnyArray` or `Ty::Any`) and turns it into the real type at the call site
/// using the arg that matched the first
/// wildcard param. builtins without a wildcard return type
/// (the overwhelming majority) pass through untouched. good.
///
/// in practice there are only two cases: `AnyArray -> AnyArray`
/// (slice/reverse/sort: the input array's own type, e.g.
/// `array<int>`, becomes the return type) and `AnyArray -> Any` (pop: the
/// input array's *element* type becomes the return type, e.g.
/// `array<int>` in, `int` out. simple math.
fn resolve_wildcard_return(declared_return: &Ty, params: &[Ty], arg_types: &[&Ty]) -> Ty {
    match declared_return {
        Ty::AnyArray => {
            // find the first AnyArray param and hand back the
            // concrete array type the caller actually passed there
            for (p, a) in params.iter().zip(arg_types.iter()) {
                if matches!(p, Ty::AnyArray) {
                    return (*a).clone();
                }
            }
            declared_return.clone()
        }
        Ty::Any => {
            // `pop`'s case: an AnyArray param's *element* type is the
            // real return type, not the array type. the one with the twist.
            for (p, a) in params.iter().zip(arg_types.iter()) {
                if matches!(p, Ty::AnyArray) {
                    if let Ty::Array(elem) = a {
                        return (**elem).clone();
                    }
                }
            }
            declared_return.clone()
        }
        other => other.clone(),
    }
}

/// type compatibility check for call args. `Any` (builtins only)
/// matches every type. `AnyArray` (builtins only) matches `array<T>` for
/// any T. every other pair must match exactly. no implicit numeric
/// coercion in v1.0. int is not float, get over it.
pub(crate) fn ty_matches(expected: &Ty, found: &Ty) -> bool {
    match expected {
        Ty::Any => true,
        Ty::AnyArray => matches!(found, Ty::Array(_)),
        Ty::AnyMap => matches!(found, Ty::Map(..)),
        _ => expected.accepts(found),
    }
}

/// true if this list of statements is guaranteed to hit a `return` on
/// every possible path, no matter the runtime values. used to
/// spot functions that fall off the end without returning. gotcha.
fn always_returns(stmts: &[Stmt], exh: &std::collections::HashSet<(Option<String>, usize, usize)>) -> bool {
    stmts.iter().any(|s| stmt_always_returns(s, exh))
}

/// the value of an integer literal, `-` included (`3`, `-3`). `None` for
/// anything that has to be computed. used to classify a range `step`.
fn literal_int(e: &Expr) -> Option<i64> {
    match e {
        Expr::IntLit(n, _) => Some(*n),
        Expr::Unary { op: UnaryOp::Neg, expr, .. } => match &**expr {
            Expr::IntLit(n, _) => n.checked_neg(),
            _ => None,
        },
        _ => None,
    }
}

fn stmt_always_returns(stmt: &Stmt, exh: &std::collections::HashSet<(Option<String>, usize, usize)>) -> bool {
    match stmt {
        Stmt::Return { .. } => true,
        // a `throw` never falls through either. dramatic exit.
        Stmt::Throw { .. } => true,

        // both branches must guarantee a return. a missing else means the
        // "skip the if entirely" path never returns. oops.
        Stmt::IfStmt { then_branch, else_branch, .. } => {
            let then_ok = always_returns(&then_branch.stmts, exh);
            let else_ok = else_branch.as_ref().map_or(false, |b| always_returns(&b.stmts, exh));
            then_ok && else_ok
        }

        // a match returns only when every arm and the `else` do (conservative. an
        // enum match covering all variants without `else` still counts as one here)
        // an enum match with no `else` is fine when it names every variant
        // (the checker writes those down in `exh`)
        Stmt::Match { arms, else_branch, pos, .. } => {
            let no_else_but_exhaustive =
                else_branch.is_none() && exh.contains(&(pos.file.clone(), pos.line, pos.col));
            (no_else_but_exhaustive || else_branch.as_ref().map_or(false, |b| always_returns(&b.stmts, exh)))
                && arms.iter().all(|a| always_returns(&a.body.stmts, exh))
        }

        // loops never guarantee a return. the body might run zero times. shrug.
        Stmt::While { .. } => false,
        Stmt::ForIn { .. } => false,
        Stmt::ForRange { .. } => false,

        // both the try body and the catch body must guarantee a return
        Stmt::Try { try_block, catch_block, .. } => {
            always_returns(&try_block.stmts, exh) && always_returns(&catch_block.stmts, exh)
        }

        _ => false,
    }
}

pub fn resolve_type_expr(t: &TypeExpr) -> Ty {
    match t {
        TypeExpr::Int => Ty::Int,
        TypeExpr::Float => Ty::Float,
        TypeExpr::Bool => Ty::Bool,
        TypeExpr::String => Ty::String,
        TypeExpr::Void => Ty::Void,
        TypeExpr::Array(inner) => Ty::Array(Box::new(resolve_type_expr(inner))),
        TypeExpr::Map(k, v) => Ty::Map(Box::new(resolve_type_expr(k)), Box::new(resolve_type_expr(v))),
        TypeExpr::Named(name) => Ty::Named(name.clone()),
        TypeExpr::Function { params, ret } => Ty::Function {
            params: params.iter().map(resolve_type_expr).collect(),
            ret: Box::new(resolve_type_expr(ret)),
        },
        TypeExpr::Optional(inner) => Ty::Optional(Box::new(resolve_type_expr(inner))),
        TypeExpr::Union(members) => {
            // nested unions are flattened and repeated members dropped: `int or int` is just `int`
            let mut flat: Vec<Ty> = Vec::new();
            for m in members {
                match resolve_type_expr(m) {
                    Ty::Union(inner) => inner.into_iter().for_each(|t| push_unique(&mut flat, t)),
                    t => push_unique(&mut flat, t),
                }
            }
            if flat.len() == 1 {
                flat.remove(0)
            } else {
                Ty::Union(flat)
            }
        }
        TypeExpr::Task(inner) => Ty::Task(Box::new(resolve_type_expr(inner))),
    }
}

/// the source location of a statement. one without its own position
/// (none today, but cheap insurance) keeps the previous one's location
fn stmt_loc(stmt: &p::Stmt, previous: &SrcLoc) -> SrcLoc {
    match stmt.pos() {
        Some(pos) => SrcLoc { file: pos.file.clone(), line: pos.line },
        None => previous.clone(),
    }
}

/// the names a condition proves aren't `none`: (when true, when false).
/// only `name == none`, `name != none`, `!`, `&&` and `||` of those say anything
/// the variable and the type an arm proves, for `when (type(x)) { "int" { ... } }`: in that arm `x` is
/// an `int`. only a single name of a basic type counts: `"array"` or several names prove nothing here.
/// a fact a condition proves about a union variable. `Is`: the variable is that member. `Not`: it is
/// not (the other members remain, and when one is left the variable is that one).
#[derive(Clone)]
enum TypeFact {
    Is(String, Ty),
    Not(String, Ty),
}

/// the basic type a type name stands for: `"int"` is `Ty::Int`. other names are not tested here.
fn basic_ty(name: &str) -> Option<Ty> {
    match name {
        "int" => Some(Ty::Int),
        "float" => Some(Ty::Float),
        "bool" => Some(Ty::Bool),
        "string" => Some(Ty::String),
        _ => None,
    }
}

/// `type(x) == "int"` or `"int" == type(x)`: the variable and the type the test names
fn type_test_pair(a: &Expr, b: &Expr) -> Option<(String, Ty)> {
    if let (Some(var), Expr::StringLit(name, _)) = (type_test_subject(a), b) {
        return basic_ty(name).map(|t| (var, t));
    }
    if let (Expr::StringLit(name, _), Some(var)) = (a, type_test_subject(b)) {
        return basic_ty(name).map(|t| (var, t));
    }
    None
}

/// the facts a condition proves about union variables when it holds (first) and when it fails (second).
/// the same shape as `narrowing`, for `type(x) == "int"` tests.
fn type_facts(cond: &Expr) -> (Vec<TypeFact>, Vec<TypeFact>) {
    match cond {
        Expr::Binary { left, op: op @ (BinaryOp::Eq | BinaryOp::NotEq), right, .. } => {
            let Some((var, ty)) = type_test_pair(left, right) else { return (Vec::new(), Vec::new()) };
            let (is, not) = (TypeFact::Is(var.clone(), ty.clone()), TypeFact::Not(var, ty));
            if matches!(op, BinaryOp::Eq) {
                (vec![is], vec![not])
            } else {
                (vec![not], vec![is])
            }
        }
        // both halves held. good boy.
        Expr::Binary { left, op: BinaryOp::And, right, .. } => ([type_facts(left).0, type_facts(right).0].concat(), Vec::new()),
        // neither half held. oof.
        Expr::Binary { left, op: BinaryOp::Or, right, .. } => (Vec::new(), [type_facts(left).1, type_facts(right).1].concat()),
        Expr::Unary { op: UnaryOp::Not, expr, .. } => {
            let (t, f) = type_facts(expr);
            (f, t)
        }
        _ => (Vec::new(), Vec::new()),
    }
}

/// the variable `x` of a `type(x)` subject, if the subject is one
fn type_test_subject(subject: &Expr) -> Option<String> {
    let Expr::Call { callee, args, .. } = subject else { return None };
    let Expr::Ident(func, _) = &**callee else { return None };
    if func != "type" || args.len() != 1 {
        return None;
    }
    let Expr::Ident(var, _) = &args[0] else { return None };
    Some(var.clone())
}

fn type_test_arm(subject: &Expr, values: &[Expr]) -> Option<(String, Ty)> {
    let var = type_test_subject(subject)?;
    let [Expr::StringLit(name, _)] = values else { return None };
    let ty = match name.as_str() {
        "int" => Ty::Int,
        "float" => Ty::Float,
        "bool" => Ty::Bool,
        "string" => Ty::String,
        _ => return None,
    };
    Some((var, ty))
}

fn narrowing(cond: &Expr) -> (Vec<String>, Vec<String>) {
    match cond {
        Expr::Binary { left, op, right, .. } => match op {
            BinaryOp::Eq | BinaryOp::NotEq => {
                let name = match (&**left, &**right) {
                    (Expr::Ident(n, _), Expr::NoneLit(_)) | (Expr::NoneLit(_), Expr::Ident(n, _)) => n.clone(),
                    _ => return (Vec::new(), Vec::new()),
                };
                if matches!(op, BinaryOp::NotEq) {
                    (vec![name], Vec::new())
                } else {
                    (Vec::new(), vec![name])
                }
            }
            // both halves held. good boy.
            BinaryOp::And => ([narrowing(left).0, narrowing(right).0].concat(), Vec::new()),
            // neither half held. oof.
            BinaryOp::Or => (Vec::new(), [narrowing(left).1, narrowing(right).1].concat()),
            _ => (Vec::new(), Vec::new()),
        },
        Expr::Unary { op: UnaryOp::Not, expr, .. } => {
            let (t, f) = narrowing(expr);
            (f, t)
        }
        _ => (Vec::new(), Vec::new()),
    }
}

/// whether control never reaches the end of this block (it ends in a return, throw,
/// break or continue). the end is not coming.
fn block_exits(block: &p::Block) -> bool {
    block.stmts.last().is_some_and(stmt_exits)
}

fn stmt_exits(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Return { .. } | Stmt::Break { .. } | Stmt::Continue { .. } | Stmt::Throw { .. } => true,
        Stmt::IfStmt { then_branch, else_branch: Some(other), .. } => block_exits(then_branch) && block_exits(other),
        Stmt::Match { arms, else_branch: Some(other), .. } => block_exits(other) && arms.iter().all(|a| block_exits(&a.body)),
        _ => false,
    }
}

/// the element type of an array literal when two items differ only by `none`
/// (`[1, none]` is an `array<int?>`). `None` when they don't, or when they're equal
fn unify_optional(a: &Ty, b: &Ty) -> Option<Ty> {
    if a == b {
        return None;
    }
    match (a, b) {
        (Ty::Optional(_), Ty::NoneLit) => Some(a.clone()),
        (Ty::NoneLit, Ty::Optional(_)) => Some(b.clone()),
        (Ty::Optional(x), other) if x.as_ref() == other => Some(a.clone()),
        (other, Ty::Optional(x)) if x.as_ref() == other => Some(b.clone()),
        (Ty::NoneLit, other) | (other, Ty::NoneLit) if !matches!(other, Ty::Void) => Some(Ty::Optional(Box::new(other.clone()))),
        _ => None,
    }
}

/// `Type.name` -> `name`
fn member_of(qualified: &str) -> &str {
    qualified.split_once('.').map_or(qualified, |(_, m)| m)
}

/// a plain name, or fields of names (`red`, `me.shape`): reading it twice has no effect
fn is_name_path(expr: &p::Expr) -> bool {
    match expr {
        p::Expr::Ident(..) => true,
        p::Expr::Field { object, .. } => is_name_path(object),
        _ => false,
    }
}

fn push_unique(list: &mut Vec<Ty>, t: Ty) {
    if !list.contains(&t) {
        list.push(t);
    }
}

/// the name of a value type's method namespace, when `ty` is one of them
fn value_type_of_ty(ty: &Ty) -> Option<&'static str> {
    match ty {
        Ty::Int => Some("int"),
        Ty::Float => Some("float"),
        Ty::Bool => Some("bool"),
        Ty::String => Some("string"),
        _ => None,
    }
}

/// the value type a method can be declared on: `fn int.x`, `fn float.x`, `fn bool.x`, `fn string.x`
fn value_type_of(name: &str) -> Option<Ty> {
    match name {
        "int" => Some(Ty::Int),
        "float" => Some(Ty::Float),
        "bool" => Some(Ty::Bool),
        "string" => Some(Ty::String),
        _ => None,
    }
}
