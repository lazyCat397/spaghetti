//! `guard` blocks of a struct body.
//!
//! A guard is a rule the struct must satisfy after every write to it. The guard
//! body reads and corrects the struct through `me`. Corrections are ordinary
//! writes inside the body, so they do not trigger the guard again.
//!
//! `guard (me.field as alias)` gives `alias` as another name for `me.field`:
//! reading it reads the field, and writing to it writes the field. It is only a
//! name. It does not add or remove anything the guard may do.
//!
//! The parser checks the rules that can be seen in the source (no `return`, no
//! method calls on `me`, only known fields, no alias that clashes with a
//! declaration in the body), and turns the blocks into two functions per struct:
//!
//! - `Name.__guard(me)`: runs every guard in order. a guard throws to refuse.
//! - `Name.__check(v)`: runs `__guard` on a new value and returns it (struct literals).
//!
//! The checker then wraps each write to a guarded struct: it saves a copy of the
//! whole struct, writes, runs `__guard`, and restores the copy if the guard throws.
use crate::ast::{Block, Expr, GuardSpec, Param, Pos, Stmt, TypeExpr};
use crate::parser::ParseError;

/// Checks every guard of one struct. `fields` are the struct's declared fields.
pub(crate) fn check_guards(struct_name: &str, fields: &[Param], guards: &[GuardSpec]) -> Result<(), ParseError> {
    let names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
    for g in guards {
        let aliases: Vec<&str> = g.target.iter().filter_map(|(_, a)| a.as_deref()).collect();
        let checker = Check { struct_name, fields: &names, aliases: &aliases };
        for (i, (field, alias)) in g.target.iter().enumerate() {
            if !names.contains(&field.as_str()) {
                return Err(checker.error(format!("struct '{}' has no field '{}'", struct_name, field), &g.pos));
            }
            if let Some(a) = alias {
                if a == "me" {
                    return Err(checker.error("'me' cannot be a guard alias".to_string(), &g.pos));
                }
                if g.target[..i].iter().any(|(_, b)| b.as_deref() == Some(a.as_str())) {
                    return Err(checker.error(format!("the alias '{}' is used twice in this guard", a), &g.pos));
                }
            }
        }
        checker.block(&g.body)?;
    }
    Ok(())
}

/// the statements of each guard, with its aliases already replaced by `me.field`
fn guard_bodies(guards: &[GuardSpec]) -> Vec<Vec<Stmt>> {
    guards
        .iter()
        .map(|g| {
            let mut inner = g.body.stmts.clone();
            for (field, alias) in &g.target {
                if let Some(alias) = alias {
                    let a = Alias { name: alias.clone(), field: field.clone() };
                    for s in inner.iter_mut() {
                        rewrite_stmt(s, &a);
                    }
                }
            }
            inner
        })
        .collect()
}

/// the fields a guard can change: the ones its bodies name through `me`. a bare `me`
/// (passed to a function, say) can reach any field, so the answer is then `None`: every field.
pub(crate) fn snapshot_fields(guards: &[GuardSpec]) -> Option<Vec<String>> {
    let mut m = Mentions { fields: Vec::new(), all: false };
    for stmts in guard_bodies(guards) {
        for s in &stmts {
            mention_stmt(s, &mut m);
        }
    }
    if m.all {
        None
    } else {
        m.fields.sort();
        m.fields.dedup();
        Some(m.fields)
    }
}

struct Mentions {
    fields: Vec<String>,
    all: bool,
}

fn mention_block(b: &Block, m: &mut Mentions) {
    for s in &b.stmts {
        mention_stmt(s, m);
    }
}

fn mention_expr(e: &Expr, m: &mut Mentions) {
    match e {
        Expr::Ident(n, _) if n == "me" => m.all = true,
        Expr::Field { object, field, .. } => {
            if is_me(object) {
                m.fields.push(field.clone());
            } else {
                mention_expr(object, m);
            }
        }
        Expr::Binary { left, right, .. } => {
            mention_expr(left, m);
            mention_expr(right, m);
        }
        Expr::Unary { expr, .. } => mention_expr(expr, m),
        Expr::IncDec { target, .. } => mention_expr(target, m),
        Expr::Call { callee, args, .. } => {
            mention_expr(callee, m);
            args.iter().for_each(|x| mention_expr(x, m));
        }
        Expr::NamedArg { value, .. } => mention_expr(value, m),
        Expr::Index { array, index, .. } => {
            mention_expr(array, m);
            mention_expr(index, m);
        }
        Expr::StructLit { fields, .. } => fields.iter().for_each(|(_, v)| mention_expr(v, m)),
        Expr::IfExpr { cond, then_branch, else_branch, .. } => {
            mention_expr(cond, m);
            mention_block(then_branch, m);
            mention_block(else_branch, m);
        }
        Expr::Lambda { body, .. } => mention_block(body, m),
        Expr::ArrayLit(items, _) => items.iter().for_each(|x| mention_expr(x, m)),
        Expr::MapLit(pairs, _) => pairs.iter().for_each(|(k, v)| {
            mention_expr(k, m);
            mention_expr(v, m);
        }),
        _ => {}
    }
}

fn mention_stmt(s: &Stmt, m: &mut Mentions) {
    match s {
        Stmt::Let { value, .. } | Stmt::Const { value, .. } => mention_expr(value, m),
        Stmt::Assign { name, value, .. } => {
            if name == "me" {
                m.all = true;
            }
            mention_expr(value, m);
        }
        Stmt::IndexAssign { array, index, value, .. } | Stmt::IndexCompoundAssign { array, index, value, .. } => {
            mention_expr(array, m);
            mention_expr(index, m);
            mention_expr(value, m);
        }
        Stmt::FieldAssign { object, field, value, .. } | Stmt::FieldCompoundAssign { object, field, value, .. } => {
            if is_me(object) {
                m.fields.push(field.clone());
            } else {
                mention_expr(object, m);
            }
            mention_expr(value, m);
        }
        Stmt::ExprStmt(e) => mention_expr(e, m),
        Stmt::Throw { value, .. } => mention_expr(value, m),
        Stmt::Return { value: Some(v), .. } => mention_expr(v, m),
        Stmt::IfStmt { cond, then_branch, else_branch, .. } => {
            mention_expr(cond, m);
            mention_block(then_branch, m);
            if let Some(b) = else_branch {
                mention_block(b, m);
            }
        }
        Stmt::While { cond, body, .. } | Stmt::DoWhile { body, cond, .. } => {
            mention_expr(cond, m);
            mention_block(body, m);
        }
        Stmt::ForIn { iterable, body, .. } => {
            mention_expr(iterable, m);
            mention_block(body, m);
        }
        Stmt::ForRange { start, end, step, body, .. } => {
            mention_expr(start, m);
            mention_expr(end, m);
            if let Some(st) = step {
                mention_expr(st, m);
            }
            mention_block(body, m);
        }
        Stmt::Match { subject, arms, else_branch, .. } => {
            mention_expr(subject, m);
            for arm in arms {
                arm.values.iter().for_each(|v| mention_expr(v, m));
                mention_block(&arm.body, m);
            }
            if let Some(b) = else_branch {
                mention_block(b, m);
            }
        }
        Stmt::Try { try_block, catch_block, .. } => {
            mention_block(try_block, m);
            mention_block(catch_block, m);
        }
        _ => {}
    }
}

/// The functions for a checked struct: `__guard` (a method, takes `me`) and
/// `__check` (an associated function, takes the value to check and returns it).
pub(crate) fn guard_functions(struct_name: &str, exported: bool, guards: &[GuardSpec], pos: &Pos) -> Vec<Stmt> {
    let mut body = Vec::new();
    for (g, inner) in guards.iter().zip(guard_bodies(guards)) {
        // one `if (true)` scope per guard, so the bodies of different guards never collide
        body.push(Stmt::IfStmt {
            cond: Expr::BoolLit(true, g.pos.clone()),
            then_branch: Block { stmts: inner, pos: g.pos.clone() },
            else_branch: None,
            pos: g.pos.clone(),
        });
    }

    let guard_fn = Stmt::FunctionDecl {
        name: format!("{}.__guard", struct_name),
        params: vec![Param { name: "me".to_string(), type_ann: TypeExpr::Named(struct_name.to_string()), default: None }],
        return_type: None,
        body: Block { stmts: body, pos: pos.clone() },
        exported,
        pos: pos.clone(),
    };

    let check_fn = Stmt::FunctionDecl {
        name: format!("{}.__check", struct_name),
        params: vec![Param { name: "v".to_string(), type_ann: TypeExpr::Named(struct_name.to_string()), default: None }],
        return_type: None,
        body: Block {
            stmts: vec![
                Stmt::ExprStmt(Expr::Call {
                    callee: Box::new(Expr::Field {
                        object: Box::new(Expr::Ident("v".to_string(), pos.clone())),
                        field: "__guard".to_string(),
                        pos: pos.clone(),
                    }),
                    args: Vec::new(),
                    pos: pos.clone(),
                }),
                Stmt::Return { value: Some(Expr::Ident("v".to_string(), pos.clone())), pos: pos.clone() },
            ],
            pos: pos.clone(),
        },
        exported,
        pos: pos.clone(),
    };

    vec![guard_fn, check_fn]
}

/// `alias` in a guard body means `me.field`. both reads and writes.
struct Alias {
    name: String,
    field: String,
}

impl Alias {
    fn me_field(&self, pos: &Pos) -> Expr {
        Expr::Field {
            object: Box::new(Expr::Ident("me".to_string(), pos.clone())),
            field: self.field.clone(),
            pos: pos.clone(),
        }
    }
}

fn rewrite_block(b: &mut Block, a: &Alias) {
    for s in b.stmts.iter_mut() {
        rewrite_stmt(s, a);
    }
}

fn rewrite_expr(e: &mut Expr, a: &Alias) {
    if let Expr::Ident(n, pos) = e {
        if *n == a.name {
            let pos = pos.clone();
            *e = a.me_field(&pos);
        }
        return;
    }
    match e {
        Expr::Binary { left, right, .. } => {
            rewrite_expr(left, a);
            rewrite_expr(right, a);
        }
        Expr::Unary { expr, .. } => rewrite_expr(expr, a),
        Expr::IncDec { target, .. } => rewrite_expr(target, a),
        Expr::Call { callee, args, .. } => {
            rewrite_expr(callee, a);
            for x in args.iter_mut() {
                rewrite_expr(x, a);
            }
        }
        Expr::NamedArg { value, .. } => rewrite_expr(value, a),
        Expr::Index { array, index, .. } => {
            rewrite_expr(array, a);
            rewrite_expr(index, a);
        }
        Expr::StructLit { fields, .. } => {
            for (_, v) in fields.iter_mut() {
                rewrite_expr(v, a);
            }
        }
        // the field name after the dot is not a variable: only the object is rewritten
        Expr::Field { object, .. } => rewrite_expr(object, a),
        Expr::IfExpr { cond, then_branch, else_branch, .. } => {
            rewrite_expr(cond, a);
            rewrite_block(then_branch, a);
            rewrite_block(else_branch, a);
        }
        Expr::Lambda { body, .. } => rewrite_block(body, a),
        Expr::ArrayLit(items, _) => {
            for x in items.iter_mut() {
                rewrite_expr(x, a);
            }
        }
        Expr::MapLit(pairs, _) => {
            for (k, v) in pairs.iter_mut() {
                rewrite_expr(k, a);
                rewrite_expr(v, a);
            }
        }
        _ => {}
    }
}

fn rewrite_stmt(s: &mut Stmt, a: &Alias) {
    match s {
        Stmt::Let { value, .. } | Stmt::Const { value, .. } => rewrite_expr(value, a),
        Stmt::Assign { name, value, pos } => {
            rewrite_expr(value, a);
            if *name == a.name {
                // `alias = v` becomes `me.field = v`
                let v = std::mem::replace(value, Expr::NoneLit(pos.clone()));
                let p = pos.clone();
                *s = Stmt::FieldAssign {
                    object: Expr::Ident("me".to_string(), p.clone()),
                    field: a.field.clone(),
                    value: v,
                    pos: p,
                };
            }
        }
        Stmt::IndexAssign { array, index, value, .. } | Stmt::IndexCompoundAssign { array, index, value, .. } => {
            rewrite_expr(array, a);
            rewrite_expr(index, a);
            rewrite_expr(value, a);
        }
        Stmt::FieldAssign { object, value, .. } | Stmt::FieldCompoundAssign { object, value, .. } => {
            rewrite_expr(object, a);
            rewrite_expr(value, a);
        }
        Stmt::ExprStmt(e) => rewrite_expr(e, a),
        Stmt::Throw { value, .. } => rewrite_expr(value, a),
        Stmt::Return { value: Some(v), .. } => rewrite_expr(v, a),
        Stmt::IfStmt { cond, then_branch, else_branch, .. } => {
            rewrite_expr(cond, a);
            rewrite_block(then_branch, a);
            if let Some(b) = else_branch {
                rewrite_block(b, a);
            }
        }
        Stmt::While { cond, body, .. } | Stmt::DoWhile { body, cond, .. } => {
            rewrite_expr(cond, a);
            rewrite_block(body, a);
        }
        Stmt::ForIn { iterable, body, .. } => {
            rewrite_expr(iterable, a);
            rewrite_block(body, a);
        }
        Stmt::ForRange { start, end, step, body, .. } => {
            rewrite_expr(start, a);
            rewrite_expr(end, a);
            if let Some(st) = step {
                rewrite_expr(st, a);
            }
            rewrite_block(body, a);
        }
        Stmt::Match { subject, arms, else_branch, .. } => {
            rewrite_expr(subject, a);
            for arm in arms.iter_mut() {
                for v in arm.values.iter_mut() {
                    rewrite_expr(v, a);
                }
                rewrite_block(&mut arm.body, a);
            }
            if let Some(b) = else_branch {
                rewrite_block(b, a);
            }
        }
        Stmt::Try { try_block, catch_block, .. } => {
            rewrite_block(try_block, a);
            rewrite_block(catch_block, a);
        }
        _ => {}
    }
}

struct Check<'a> {
    struct_name: &'a str,
    fields: &'a [&'a str],
    /// the aliases of this guard: none of them may be declared again inside the body
    aliases: &'a [&'a str],
}

impl Check<'_> {
    fn error(&self, reason: String, pos: &Pos) -> ParseError {
        ParseError::GuardError {
            struct_name: self.struct_name.to_string(),
            reason,
            line: pos.line,
            col: pos.col,
        }
    }

    /// a declaration named like the alias would shadow `me.field` in the body
    fn bind(&self, name: &str, pos: &Pos) -> Result<(), ParseError> {
        if self.aliases.contains(&name) {
            return Err(self.error(format!("'{}' is the guard's alias: pick another name for this declaration", name), pos));
        }
        Ok(())
    }

    fn field_access(&self, object: &Expr, field: &str, pos: &Pos) -> Result<(), ParseError> {
        if is_me(object) && !self.fields.contains(&field) {
            return Err(self.error(format!("struct '{}' has no field '{}'", self.struct_name, field), pos));
        }
        Ok(())
    }

    fn block(&self, b: &Block) -> Result<(), ParseError> {
        b.stmts.iter().try_for_each(|s| self.stmt(s))
    }

    fn stmt(&self, s: &Stmt) -> Result<(), ParseError> {
        match s {
            Stmt::Let { name, value, pos, .. } | Stmt::Const { name, value, pos, .. } => {
                self.bind(name, pos)?;
                self.expr(value)
            }
            Stmt::ExprStmt(e) => self.expr(e),
            Stmt::Throw { value, .. } => self.expr(value),
            Stmt::Return { pos, .. } => Err(self.error("a guard cannot use 'return'".to_string(), pos)),
            Stmt::Assign { name, value, pos } => {
                if name == "me" {
                    return Err(self.error("a guard cannot write to 'me'".to_string(), pos));
                }
                self.expr(value)
            }
            Stmt::IndexAssign { array, index, value, .. } | Stmt::IndexCompoundAssign { array, index, value, .. } => {
                self.expr(array)?;
                self.expr(index)?;
                self.expr(value)
            }
            Stmt::FieldAssign { object, field, value, pos } | Stmt::FieldCompoundAssign { object, field, value, pos, .. } => {
                self.field_access(object, field, pos)?;
                self.expr(object)?;
                self.expr(value)
            }
            Stmt::IfStmt { cond, then_branch, else_branch, .. } => {
                self.expr(cond)?;
                self.block(then_branch)?;
                else_branch.as_ref().map_or(Ok(()), |b| self.block(b))
            }
            Stmt::While { cond, body, .. } | Stmt::DoWhile { body, cond, .. } => {
                self.expr(cond)?;
                self.block(body)
            }
            Stmt::ForIn { var_name, index_var, iterable, body, pos } => {
                self.bind(var_name, pos)?;
                if let Some(i) = index_var {
                    self.bind(i, pos)?;
                }
                self.expr(iterable)?;
                self.block(body)
            }
            Stmt::ForRange { var_name, start, end, step, body, pos } => {
                self.bind(var_name, pos)?;
                self.expr(start)?;
                self.expr(end)?;
                if let Some(st) = step {
                    self.expr(st)?;
                }
                self.block(body)
            }
            Stmt::Match { subject, arms, else_branch, .. } => {
                self.expr(subject)?;
                for arm in arms {
                    arm.values.iter().try_for_each(|v| self.expr(v))?;
                    self.block(&arm.body)?;
                }
                else_branch.as_ref().map_or(Ok(()), |b| self.block(b))
            }
            Stmt::Try { catch_var, try_block, catch_block, pos, .. } => {
                self.bind(catch_var, pos)?;
                self.block(try_block)?;
                self.block(catch_block)
            }
            _ => Ok(()),
        }
    }

    fn expr(&self, e: &Expr) -> Result<(), ParseError> {
        match e {
            Expr::Binary { left, right, .. } => {
                self.expr(left)?;
                self.expr(right)
            }
            Expr::Unary { expr, .. } => self.expr(expr),
            Expr::IncDec { target, .. } => self.expr(target),
            Expr::Call { callee, args, pos } => {
                if let Expr::Field { object, .. } = &**callee {
                    if is_me(object) {
                        return Err(self.error("a guard cannot call methods on 'me'".to_string(), pos));
                    }
                }
                self.expr(callee)?;
                args.iter().try_for_each(|a| self.expr(a))
            }
            // the name of a named arg is a parameter; the value is checked like any argument
            Expr::NamedArg { value, .. } => self.expr(value),
            Expr::Index { array, index, .. } => {
                self.expr(array)?;
                self.expr(index)
            }
            Expr::StructLit { fields, .. } => fields.iter().try_for_each(|(_, v)| self.expr(v)),
            Expr::Field { object, field, pos } => {
                self.field_access(object, field, pos)?;
                self.expr(object)
            }
            Expr::IfExpr { cond, then_branch, else_branch, .. } => {
                self.expr(cond)?;
                self.block(then_branch)?;
                self.block(else_branch)
            }
            Expr::Lambda { params, body, .. } => {
                for p in params {
                    self.bind(&p.name, &body.pos)?;
                }
                self.block(body)
            }
            Expr::ArrayLit(items, _) => items.iter().try_for_each(|i| self.expr(i)),
            Expr::MapLit(pairs, _) => pairs.iter().try_for_each(|(k, v)| {
                self.expr(k)?;
                self.expr(v)
            }),
            _ => Ok(()),
        }
    }
}

fn is_me(e: &Expr) -> bool {
    matches!(e, Expr::Ident(n, _) if n == "me")
}
