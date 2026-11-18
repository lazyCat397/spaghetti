//! `guard` blocks of a struct body.
//!
//! A guard is a rule the struct must satisfy after every write to it. The parser
//! checks the rules that can be seen in the source (no writes to `me`, no `return`,
//! no method calls on `me`, only known fields, a field guard reads only its field),
//! and turns the blocks into two functions per struct:
//!
//! - `Name.__guard(me)`: runs every guard in order. a guard throws to refuse.
//! - `Name.__check(v)`: runs `__guard` on a new value and returns it (struct literals).
//!
//! The checker then wraps each write to a guarded struct: it saves the old value,
//! writes, runs `__guard`, and restores the old value if the guard throws.
use crate::ast::{Block, Expr, GuardSpec, Param, Pos, Stmt, TypeExpr};
use crate::parser::ParseError;

/// Checks every guard of one struct. `fields` are the struct's declared fields.
pub(crate) fn check_guards(struct_name: &str, fields: &[Param], guards: &[GuardSpec]) -> Result<(), ParseError> {
    let names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
    for g in guards {
        let only_field = g.target.as_ref().map(|(f, _)| f.as_str());
        let checker = Check { struct_name, fields: &names, only_field };
        if let Some((field, _)) = &g.target {
            if !names.contains(&field.as_str()) {
                return Err(checker.error(format!("struct '{}' has no field '{}'", struct_name, field), &g.pos));
            }
        }
        checker.block(&g.body)?;
    }
    Ok(())
}

/// The functions for a checked struct: `__guard` (a method, takes `me`) and
/// `__check` (an associated function, takes the value to check and returns it).
pub(crate) fn guard_functions(struct_name: &str, exported: bool, guards: &[GuardSpec], pos: &Pos) -> Vec<Stmt> {
    let mut body = Vec::new();
    for g in guards {
        let mut inner = Vec::new();
        if let Some((field, Some(alias))) = &g.target {
            inner.push(Stmt::Let {
                name: alias.clone(),
                type_ann: None,
                value: Expr::Field {
                    object: Box::new(Expr::Ident("me".to_string(), g.pos.clone())),
                    field: field.clone(),
                    pos: g.pos.clone(),
                },
                exported: false,
                pos: g.pos.clone(),
            });
        }
        inner.extend(g.body.stmts.iter().cloned());
        // one `if (true)` scope per guard, so aliases of different guards never collide
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

struct Check<'a> {
    struct_name: &'a str,
    fields: &'a [&'a str],
    /// set inside `guard (me.field)`: the only field the block may read
    only_field: Option<&'a str>,
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

    fn write_to_me(&self, target: &Expr, pos: &Pos) -> Result<(), ParseError> {
        if root_is_me(target) {
            return Err(self.error("a guard cannot write to 'me'".to_string(), pos));
        }
        Ok(())
    }

    fn field_access(&self, object: &Expr, field: &str, pos: &Pos) -> Result<(), ParseError> {
        if !is_me(object) {
            return Ok(());
        }
        if !self.fields.contains(&field) {
            return Err(self.error(format!("struct '{}' has no field '{}'", self.struct_name, field), pos));
        }
        if let Some(only) = self.only_field {
            if field != only {
                return Err(self.error(format!("this guard reads only 'me.{}', found 'me.{}'", only, field), pos));
            }
        }
        Ok(())
    }

    fn block(&self, b: &Block) -> Result<(), ParseError> {
        b.stmts.iter().try_for_each(|s| self.stmt(s))
    }

    fn stmt(&self, s: &Stmt) -> Result<(), ParseError> {
        match s {
            Stmt::Let { value, .. } | Stmt::Const { value, .. } => self.expr(value),
            Stmt::ExprStmt(e) => self.expr(e),
            Stmt::Throw { value, .. } => self.expr(value),
            Stmt::Return { pos, .. } => Err(self.error("a guard cannot use 'return'".to_string(), pos)),
            Stmt::Assign { name, value, pos } => {
                if name == "me" {
                    return Err(self.error("a guard cannot write to 'me'".to_string(), pos));
                }
                self.expr(value)
            }
            Stmt::IndexAssign { array, index, value, pos } => {
                self.write_to_me(array, pos)?;
                self.expr(array)?;
                self.expr(index)?;
                self.expr(value)
            }
            Stmt::IndexCompoundAssign { array, index, value, pos, .. } => {
                self.write_to_me(array, pos)?;
                self.expr(array)?;
                self.expr(index)?;
                self.expr(value)
            }
            Stmt::FieldAssign { object, field, value, pos } => {
                self.write_to_me(object, pos)?;
                self.field_access(object, field, pos)?;
                self.expr(object)?;
                self.expr(value)
            }
            Stmt::FieldCompoundAssign { object, field, value, pos, .. } => {
                self.write_to_me(object, pos)?;
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
            Stmt::ForIn { iterable, body, .. } => {
                self.expr(iterable)?;
                self.block(body)
            }
            Stmt::ForRange { start, end, step, body, .. } => {
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
            Stmt::Try { try_block, catch_block, .. } => {
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
            Expr::IncDec { target, pos, .. } => {
                self.write_to_me(target, pos)?;
                self.expr(target)
            }
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
            Expr::Lambda { body, .. } => self.block(body),
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

/// `me`, `me.a`, `me.a[i].b`... : a write through this target changes `me`
fn root_is_me(e: &Expr) -> bool {
    match e {
        Expr::Ident(n, _) => n == "me",
        Expr::Field { object, .. } => root_is_me(object),
        Expr::Index { array, .. } => root_is_me(array),
        _ => false,
    }
}
