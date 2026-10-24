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

use crate::builtins::{builtin_signatures, method_namespace, BuiltinId};
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

pub struct Checker {
    functions: HashMap<String, FunctionSig>,
    /// struct name -> its fields in declaration order (order matters for
    /// error messages, and maybe a future positional-literal
    /// syntax (even though today's `Name { field: value }` is
    /// checked by name, not position). future problems, future us.
    struct_defs: HashMap<String, Vec<Param>>,
    /// enum name -> its variants in declaration order (a variant's position
    /// is its identity at runtime). an enum value's type is `Ty::Named(name)`;
    /// this table is how it's told apart from a struct.
    enum_defs: HashMap<String, Vec<String>>,
    /// enum name -> the type of its variants' values (`c.value`)
    enum_value_tys: HashMap<String, Ty>,
    scopes: Vec<HashMap<String, VarInfo>>,
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
        struct_defs: HashMap::new(),
        enum_defs: HashMap::new(),
        enum_value_tys: HashMap::new(),
        scopes: vec![HashMap::new()],
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
                Stmt::StructDecl { name, fields, .. } => {
                    self.struct_defs.insert(name.clone(), fields.clone());
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
            if let Stmt::FunctionDecl { name, params, pos, .. } = stmt {
                if let Some((type_name, _)) = name.split_once('.') {
                    let is_method = params.first().map_or(false, |p| p.name == "me");
                    if !is_method {
                        if !self.is_type_name(type_name) {
                            self.errors.push(TypeError::AssociatedOnUnknownType {
                                type_name: type_name.to_string(),
                                name: name.clone(),
                                pos: pos.clone(),
                            });
                        } else if self.enum_defs.get(type_name).map_or(false, |vs| vs.iter().any(|v| v == member_of(name))) {
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
            if !native.methods.contains(&method) {
                self.errors.push(TypeError::NoSuchMethod {
                    struct_name: native.name.to_string(),
                    method: method.to_string(),
                    is_field: native.fields.iter().any(|f| f.name == method),
                    pos: pos.clone(),
                });
                return None;
            }
            let mut typed_args = vec![typed_object];
            for arg in args {
                typed_args.push(self.check_expr(arg)?);
            }
            return self.lib_call(module, method, typed_args, pos);
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
        if let Some(ns) = method_namespace(&receiver_ty, method) {
            return self.check_native_method(typed_object, ns, method, args, pos);
        }
        if matches!(receiver_ty, Ty::String | Ty::Array(_) | Ty::Map(..) | Ty::Int | Ty::Float) {
            self.errors.push(TypeError::NoSuchNativeMethod {
                found: receiver_ty,
                method: method.to_string(),
                pos: pos.clone(),
            });
            return None;
        }
        let struct_name = match typed_object.ty() {
            Ty::Named(n) => n.clone(),
            other => {
                self.errors.push(TypeError::NoMethodsOnType {
                    found: other.clone(),
                    method: method.to_string(),
                    pos: pos.clone(),
                });
                return None;
            }
        };

        let mut typed_args = vec![typed_object];
        for arg in args {
            typed_args.push(self.check_expr(arg)?);
        }

        let full_name = format!("{}.{}", struct_name, method);
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

    fn check_stmt(&mut self, stmt: &Stmt) -> Option<TypedStmt> {
        match stmt {
            Stmt::Let { name, type_ann, value, exported, pos } => {
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

                self.push_scope();
                self.narrow(&when_true);
                let typed_then = self.check_block(then_branch);
                self.pop_scope();

                let typed_else = else_branch.as_ref().map(|b| {
                    self.push_scope();
                    self.narrow(&when_false);
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
                } else if else_exits && !then_exits {
                    self.narrow(&when_true);
                }

                Some(TypedStmt::IfStmt { cond: typed_cond, then_branch: typed_then, else_branch: typed_else })
            }

            Stmt::While { cond, body, pos } => {
                let typed_cond = self.check_expr(cond)?;
                if typed_cond.ty() != &Ty::Bool {
                    self.errors.push(TypeError::NonBoolCondition { found: typed_cond.ty().clone(), pos: pos.clone() });
                }
                let (when_true, _) = narrowing(cond);
                self.push_scope();
                self.narrow(&when_true);
                self.loop_depth += 1;
                let typed_body = self.check_block(body);
                self.loop_depth -= 1;
                self.pop_scope();
                Some(TypedStmt::While { cond: typed_cond, body: typed_body })
            }

            Stmt::ForIn { var_name, iterable, body, pos } => {
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
                let elem_ty = match typed_iterable.ty() {
                    Ty::Array(inner) => (**inner).clone(),
                    other => {
                        self.errors.push(TypeError::NotIterable { found: other.clone(), pos: pos.clone() });
                        Ty::Void
                    }
                };

                self.push_scope();
                self.declare(var_name, elem_ty.clone(), true, pos);
                self.loop_depth += 1;
                let typed_body = self.check_block(body);
                self.loop_depth -= 1;
                self.pop_scope();

                Some(TypedStmt::ForIn {
                    var_name: var_name.clone(),
                    elem_ty,
                    iterable: typed_iterable,
                    body: typed_body,
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
                    Some(v) => Some(self.check_expr(v)?),
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

            Stmt::Import { names, from, .. } => {
                Some(TypedStmt::Import { names: names.clone(), from: from.clone() })
            }

            // consumed by spg-modules along with every `Import`. one
            // reaching the checker means the resolver skipped it. resolver, explain yourself.
            Stmt::ReExport { .. } => {
                unreachable!("compiler bug: `export {{ .. }} from` reached the typechecker (spg-modules should have consumed it)")
            }
            Stmt::Install { .. } => {
                unreachable!("compiler bug: `install` reached the typechecker (spg-modules should have consumed it)")
            }

            Stmt::Assign { name, value, pos } => {
                let typed_value = self.check_expr(value)?;
                let found_ty = typed_value.ty().clone();

                match self.lookup_var(name) {
                    None => {
                        if spg_lexer::is_install_namespace(name.as_str()) {
                        // a bare `http` / `ps` (no `.member`): say what it is instead of "undefined"
                        self.errors.push(TypeError::NeedsInstall { lib: name.to_string(), pos: pos.clone() });
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
                    if !is_name_path(object) {
                        self.errors.push(TypeError::CompoundOnNativeNeedsName {
                            struct_name: native.name.to_string(),
                            field: field.clone(),
                            pos: pos.clone(),
                        });
                        return None;
                    }
                    let current = self.lib_call(module, def.get, vec![typed_object.clone()], pos)?;
                    let result_ty = self.binary_result_type(op, current.ty(), typed_value.ty(), pos)?;
                    let next = TypedExpr::Binary {
                        left: Box::new(current),
                        op: op.clone(),
                        right: Box::new(typed_value),
                        ty: result_ty,
                    };
                    let call = self.lib_call(module, setter, vec![typed_object, next], pos)?;
                    return Some(TypedStmt::ExprStmt(call));
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

    fn check_expr(&mut self, expr: &Expr) -> Option<TypedExpr> {
        match expr {
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
                        self.errors.push(TypeError::NeedsInstall { lib: name.to_string(), pos: pos.clone() });
                    } else {
                        self.errors.push(TypeError::UndefinedVariable { name: name.clone(), pos: pos.clone() });
                    }
                    None
                }
            }

            Expr::Binary { left, op, right, pos } => {
                let mut typed_left = self.check_expr(left)?;
                // `a != none && a.x > 0`: the right side sees what the left side proved
                let proven = match op {
                    BinaryOp::And => narrowing(left).0,
                    BinaryOp::Or => narrowing(left).1,
                    _ => Vec::new(),
                };
                let checked_right = if proven.is_empty() {
                    self.check_expr(right)
                } else {
                    self.push_scope();
                    self.narrow(&proven);
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
                    // `ps.run(...)` with no `install #ps` (with it, the lexer already made
                    // it a namespace call). tell them what's missing instead of "undefined ps"
                    Expr::Field { object, .. }
                        if matches!(object.as_ref(), Expr::Ident(ns, _)
                            if spg_lexer::is_install_namespace(ns) && self.lookup_var(ns).is_none()) =>
                    {
                        let lib = match object.as_ref() {
                            Expr::Ident(ns, _) => ns.clone(),
                            _ => unreachable!(),
                        };
                        self.errors.push(TypeError::NeedsInstall { lib, pos: pos.clone() });
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
                let mut typed_args = Vec::new();
                for arg in args {
                    typed_args.push(self.check_expr(arg)?);
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
                    let typed_value = self.check_expr(value_expr)?;
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

                self.push_scope();
                self.narrow(&when_true);
                let typed_then = self.check_block(then_branch);
                self.pop_scope();

                self.push_scope();
                self.narrow(&when_false);
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
