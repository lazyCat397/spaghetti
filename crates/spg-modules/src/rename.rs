//! Rewrites symbol names across a whole module's AST.
//!
//! Every top-level symbol of a module gets a globally unique "mangled"
//! name, and every USE of a symbol — inside that module or, via an
//! import, inside another one — is rewritten to that mangled name. After
//! this pass runs on every module, the merged program has no name
//! collisions, so the typechecker/compiler never need to know files
//! existed.
//!
//! The one subtlety is shadowing: a function parameter or a `let` inside
//! a block can legitimately reuse a name that ALSO exists at module
//! level. Renaming that use would silently retarget it at the wrong
//! variable, so the walk tracks which names are currently shadowed by a
//! local and leaves those alone. This mirrors the typechecker's own
//! block-scoping rule exactly.

use std::collections::{HashMap, HashSet};

use spg_parser::ast::{Block, Expr, Param, Stmt, TypeExpr};

/// `original name -> mangled name` for everything a module can see
/// (its own top-level symbols plus whatever it imported)
pub type NameMap = HashMap<String, String>;

pub struct Renamer<'a> {
    map: &'a NameMap,
    /// names that two `install`s provide with DIFFERENT meanings: harmless
    /// until something uses them, so a use is recorded in `hits`
    ambiguous: HashSet<String>,
    hits: std::cell::RefCell<Vec<String>>,
    /// one set per open block; a name in ANY of them is a local that
    /// shadows the module-level symbol of the same name
    shadowed: Vec<HashSet<String>>,
}

impl<'a> Renamer<'a> {
    pub fn new(map: &'a NameMap) -> Self {
        Renamer { map, ambiguous: HashSet::new(), hits: std::cell::RefCell::new(Vec::new()), shadowed: Vec::new() }
    }

    pub fn with_ambiguous(mut self, names: HashSet<String>) -> Self {
        self.ambiguous = names;
        self
    }

    /// the ambiguous names that were actually used, first use first
    pub fn used_ambiguous(&self) -> Vec<String> {
        self.hits.borrow().clone()
    }

    fn note_use(&self, name: &str) {
        if self.ambiguous.contains(name) {
            let mut hits = self.hits.borrow_mut();
            if !hits.iter().any(|h| h == name) {
                hits.push(name.to_string());
            }
        }
    }

    fn is_shadowed(&self, name: &str) -> bool {
        self.shadowed.iter().any(|s| s.contains(name))
    }

    fn shadow(&mut self, name: &str) {
        if let Some(top) = self.shadowed.last_mut() {
            top.insert(name.to_string());
        }
    }

    /// resolves a VALUE-position name (variable/function): shadowed
    /// locals keep their own name, module-level symbols get mangled
    fn value_name(&self, name: &str) -> String {
        if self.is_shadowed(name) {
            return name.to_string();
        }
        self.note_use(name);
        self.map.get(name).cloned().unwrap_or_else(|| name.to_string())
    }

    /// resolves a TYPE-position name. Types live in their own namespace
    /// (a local variable can never shadow a struct name), so no shadow
    /// check here.
    fn type_name(&self, name: &str) -> String {
        self.note_use(name);
        self.map.get(name).cloned().unwrap_or_else(|| name.to_string())
    }

    // ---- types ----

    pub fn rename_type(&self, ty: &mut TypeExpr) {
        match ty {
            TypeExpr::Named(n) => *n = self.type_name(n),
            TypeExpr::Array(inner) => self.rename_type(inner),
            TypeExpr::Map(key, value) => {
                self.rename_type(key);
                self.rename_type(value);
            }
            TypeExpr::Optional(inner) => self.rename_type(inner),
            TypeExpr::Function { params, ret } => {
                for p in params {
                    self.rename_type(p);
                }
                self.rename_type(ret);
            }
            _ => {}
        }
    }

    fn rename_params(&self, params: &mut [Param]) {
        for p in params {
            self.rename_type(&mut p.type_ann);
        }
    }

    // ---- statements ----

    /// Renames a whole module body. Top-level declarations' OWN names are
    /// renamed too (they're in `map` by construction), so a declaration
    /// and every use of it stay in sync.
    pub fn rename_program(&mut self, stmts: &mut [Stmt]) {
        // top-level is the module's outermost block: locals declared
        // here are module-level symbols (already in `map`), NOT shadows,
        // so no shadow frame is pushed for it
        for stmt in stmts {
            self.rename_stmt(stmt, true);
        }
    }

    fn rename_block(&mut self, block: &mut Block) {
        self.shadowed.push(HashSet::new());
        for stmt in &mut block.stmts {
            self.rename_stmt(stmt, false);
        }
        self.shadowed.pop();
    }

    fn rename_stmt(&mut self, stmt: &mut Stmt, top_level: bool) {
        match stmt {
            Stmt::Let { name, type_ann, value, .. } | Stmt::Const { name, type_ann, value, .. } => {
                // the initializer is evaluated BEFORE the new name comes
                // into scope, so `let x = x + 1` reads the outer `x`
                self.rename_expr(value);
                if let Some(t) = type_ann {
                    self.rename_type(t);
                }
                if top_level {
                    *name = self.value_name(name);
                } else {
                    self.shadow(name);
                }
            }

            Stmt::Assign { name, value, .. } => {
                self.rename_expr(value);
                *name = self.value_name(name);
            }

            Stmt::IndexAssign { array, index, value, .. } => {
                self.rename_expr(array);
                self.rename_expr(index);
                self.rename_expr(value);
            }
            Stmt::IndexCompoundAssign { array, index, value, .. } => {
                self.rename_expr(array);
                self.rename_expr(index);
                self.rename_expr(value);
            }
            Stmt::FieldAssign { object, value, .. } => {
                self.rename_expr(object);
                self.rename_expr(value);
            }
            Stmt::FieldCompoundAssign { object, value, .. } => {
                self.rename_expr(object);
                self.rename_expr(value);
            }

            Stmt::Match { subject, arms, else_branch, .. } => {
                self.rename_expr(subject);
                for arm in arms.iter_mut() {
                    for v in arm.values.iter_mut() {
                        self.rename_expr(v);
                    }
                    self.rename_block(&mut arm.body);
                }
                if let Some(b) = else_branch {
                    self.rename_block(b);
                }
            }

            Stmt::IfStmt { cond, then_branch, else_branch, .. } => {
                self.rename_expr(cond);
                self.rename_block(then_branch);
                if let Some(b) = else_branch {
                    self.rename_block(b);
                }
            }

            Stmt::While { cond, body, .. } => {
                self.rename_expr(cond);
                self.rename_block(body);
            }

            Stmt::ForIn { var_name, iterable, body, .. } => {
                self.rename_expr(iterable);
                // the loop variable is visible only inside the body
                self.shadowed.push(HashSet::new());
                self.shadow(var_name);
                self.rename_block(body);
                self.shadowed.pop();
            }

            Stmt::ForRange { var_name, start, end, step, body, .. } => {
                // start/end/step are evaluated OUTSIDE the loop variable's scope
                self.rename_expr(start);
                self.rename_expr(end);
                if let Some(s) = step {
                    self.rename_expr(s);
                }
                self.shadowed.push(HashSet::new());
                self.shadow(var_name);
                self.rename_block(body);
                self.shadowed.pop();
            }

            Stmt::Return { value, .. } => {
                if let Some(v) = value {
                    self.rename_expr(v);
                }
            }

            Stmt::Try { try_block, catch_var, catch_type, catch_block, .. } => {
                self.rename_block(try_block);
                if let Some(t) = catch_type {
                    self.rename_type(t);
                }
                self.shadowed.push(HashSet::new());
                self.shadow(catch_var);
                self.rename_block(catch_block);
                self.shadowed.pop();
            }

            Stmt::FunctionDecl { name, params, return_type, body, .. } => {
                *name = self.value_name(name);
                self.rename_params(params);
                if let Some(rt) = return_type {
                    self.rename_type(rt);
                }
                // parameters shadow module-level names inside the body
                self.shadowed.push(HashSet::new());
                for p in params.iter() {
                    self.shadow(&p.name);
                }
                self.rename_block(body);
                self.shadowed.pop();
            }

            Stmt::StructDecl { name, fields, .. } => {
                *name = self.type_name(name);
                self.rename_params(fields);
            }

            // the enum's own name is mangled like a struct's; its variants are
            // only ever reached as `Enum.Variant`, so they never need renaming
            Stmt::EnumDecl { name, .. } => *name = self.type_name(name),

            // imports are consumed by the resolver before renaming; by
            // the time a module body reaches this pass they're gone
            Stmt::Import { .. } | Stmt::ReExport { .. } | Stmt::Install { .. } => {}

            Stmt::Break { .. } | Stmt::Continue { .. } => {}
            Stmt::Throw { value, .. } => self.rename_expr(value),

            Stmt::ExprStmt(e) => self.rename_expr(e),
        }
    }

    // ---- expressions ----

    fn rename_expr(&mut self, expr: &mut Expr) {
        match expr {
            Expr::IntLit(..) | Expr::FloatLit(..) | Expr::StringLit(..) | Expr::BoolLit(..) | Expr::NoneLit(..) => {}

            Expr::ArrayLit(items, _) => {
                for i in items {
                    self.rename_expr(i);
                }
            }

            Expr::MapLit(entries, _) => {
                for (k, v) in entries {
                    self.rename_expr(k);
                    self.rename_expr(v);
                }
            }

            Expr::Ident(name, _) => *name = self.value_name(name),

            // a lambda's parameters shadow module-level names inside its body,
            // exactly like a function declaration's do
            Expr::Lambda { params, return_type, body, .. } => {
                self.rename_params(params);
                if let Some(rt) = return_type {
                    self.rename_type(rt);
                }
                self.shadowed.push(HashSet::new());
                for p in params.iter() {
                    self.shadow(&p.name);
                }
                self.rename_block(body);
                self.shadowed.pop();
            }

            Expr::Binary { left, right, .. } => {
                self.rename_expr(left);
                self.rename_expr(right);
            }

            Expr::Unary { expr, .. } => self.rename_expr(expr),

            Expr::Call { callee, args, .. } => {
                self.rename_expr(callee);
                for a in args {
                    self.rename_expr(a);
                }
            }

            Expr::Index { array, index, .. } => {
                self.rename_expr(array);
                self.rename_expr(index);
            }

            Expr::StructLit { name, fields, .. } => {
                *name = self.type_name(name);
                for (_, v) in fields {
                    self.rename_expr(v);
                }
            }

            Expr::Field { object, .. } => self.rename_expr(object),

            Expr::IfExpr { cond, then_branch, else_branch, .. } => {
                self.rename_expr(cond);
                self.rename_block(then_branch);
                self.rename_block(else_branch);
            }
        }
    }
}
