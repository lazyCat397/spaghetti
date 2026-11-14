//! stamps every `Pos` in a freshly-parsed module with the file it came
//! from, before that module's statements get merged into the program.
//!
//! the parser never sets `Pos.file`. it only sees tokens, not a
//! filename, so every position starts life as `file: None`. that's
//! exactly right for a single-file program (no filename needed,
//! so no diagnostic ever prints one). it stops being right once several
//! files are merged into one `Program`. a `Pos` with only a line/column
//! is ambiguous the moment it could belong to any of them. this module
//! is the one place that resolves that ambiguity, once, right after each
//! module gets parsed.
//!
//! the walk mirrors `rename.rs` on purpose. same tree, same shape,
//! but they stay separate: renaming rewrites NAMES and only
//! touches a few `Expr`/`Stmt` fields that hold one, so it never
//! had to (and still doesn't) look at `pos`. this walks EVERY
//! node for its `pos` and nothing else. folding one into the other would
//! make both harder to read, for no shared logic. no merge, thanks.

use spg_parser::ast::{Block, Expr, Param, Pos, Stmt, TypeExpr};

/// stamps `file` onto every `Pos` reachable from `stmts`.
pub fn annotate_file(stmts: &mut [Stmt], file: &str) {
    for stmt in stmts {
        annotate_stmt(stmt, file);
    }
}

fn stamp(pos: &mut Pos, file: &str) {
    pos.file = Some(file.to_string());
}

fn annotate_block(block: &mut Block, file: &str) {
    stamp(&mut block.pos, file);
    for stmt in &mut block.stmts {
        annotate_stmt(stmt, file);
    }
}

fn annotate_type(ty: &mut TypeExpr, _file: &str) {
    // TypeExpr carries no Pos of its own (see ast.rs). nothing to stamp.
    // kept as a function anyway, so a future field in TypeExpr
    // has an obvious place to be wired in, same as `rename_type`
    // does for names. consistency, again.
    let _ = ty;
}

fn annotate_params(params: &mut [Param], file: &str) {
    for p in params {
        annotate_type(&mut p.type_ann, file);
    }
}

fn annotate_stmt(stmt: &mut Stmt, file: &str) {
    match stmt {
        Stmt::Let { type_ann, value, pos, .. } | Stmt::Const { type_ann, value, pos, .. } => {
            stamp(pos, file);
            if let Some(t) = type_ann {
                annotate_type(t, file);
            }
            annotate_expr(value, file);
        }

        Stmt::Match { subject, arms, else_branch, pos } => {
            stamp(pos, file);
            annotate_expr(subject, file);
            for arm in arms.iter_mut() {
                stamp(&mut arm.pos, file);
                for v in arm.values.iter_mut() {
                    annotate_expr(v, file);
                }
                annotate_block(&mut arm.body, file);
            }
            if let Some(b) = else_branch {
                annotate_block(b, file);
            }
        }

        Stmt::IfStmt { cond, then_branch, else_branch, pos } => {
            stamp(pos, file);
            annotate_expr(cond, file);
            annotate_block(then_branch, file);
            if let Some(b) = else_branch {
                annotate_block(b, file);
            }
        }

        Stmt::CompileVm { body, pos } | Stmt::CompileJit { body, pos } => {
            stamp(pos, file);
            annotate_block(body, file);
        }

        Stmt::DoWhile { body, cond, pos } => {
            stamp(pos, file);
            annotate_block(body, file);
            annotate_expr(cond, file);
        }

        Stmt::While { cond, body, pos } => {
            stamp(pos, file);
            annotate_expr(cond, file);
            annotate_block(body, file);
        }

        Stmt::ForIn { iterable, body, pos, .. } => {
            stamp(pos, file);
            annotate_expr(iterable, file);
            annotate_block(body, file);
        }

        Stmt::ForRange { start, end, step, body, pos, .. } => {
            stamp(pos, file);
            annotate_expr(start, file);
            annotate_expr(end, file);
            if let Some(s) = step {
                annotate_expr(s, file);
            }
            annotate_block(body, file);
        }

        Stmt::Return { value, pos } => {
            stamp(pos, file);
            if let Some(v) = value {
                annotate_expr(v, file);
            }
        }

        Stmt::Break { pos } | Stmt::Continue { pos } => stamp(pos, file),
        Stmt::Throw { value, pos } => {
            stamp(pos, file);
            annotate_expr(value, file);
        }

        Stmt::Try { try_block, catch_type, catch_block, pos, .. } => {
            stamp(pos, file);
            annotate_block(try_block, file);
            if let Some(t) = catch_type {
                annotate_type(t, file);
            }
            annotate_block(catch_block, file);
        }

        Stmt::FunctionDecl { params, return_type, body, pos, .. } => {
            stamp(pos, file);
            annotate_params(params, file);
            if let Some(rt) = return_type {
                annotate_type(rt, file);
            }
            annotate_block(body, file);
        }

        Stmt::StructDecl { fields, pos, .. } => {
            stamp(pos, file);
            annotate_params(fields, file);
        }

        Stmt::EnumDecl { pos, .. } => stamp(pos, file),

        // consumed by spg-modules before this walk ever gets to the
        // typechecker/compiler, but annotated anyway, in case a future
        // error (e.g. resolving the `from` path) wants their own pos
        Stmt::Use { pos, .. } | Stmt::ReExport { pos, .. } | Stmt::Load { pos, .. } => stamp(pos, file),

        Stmt::Assign { value, pos, .. } => {
            stamp(pos, file);
            annotate_expr(value, file);
        }

        Stmt::IndexAssign { array, index, value, pos } => {
            stamp(pos, file);
            annotate_expr(array, file);
            annotate_expr(index, file);
            annotate_expr(value, file);
        }
        Stmt::IndexCompoundAssign { array, index, value, pos, .. } => {
            stamp(pos, file);
            annotate_expr(array, file);
            annotate_expr(index, file);
            annotate_expr(value, file);
        }
        Stmt::FieldAssign { object, value, pos, .. } => {
            stamp(pos, file);
            annotate_expr(object, file);
            annotate_expr(value, file);
        }
        Stmt::FieldCompoundAssign { object, value, pos, .. } => {
            stamp(pos, file);
            annotate_expr(object, file);
            annotate_expr(value, file);
        }

        Stmt::ExprStmt(e) => annotate_expr(e, file),
    }
}

fn annotate_expr(expr: &mut Expr, file: &str) {
    match expr {
        Expr::IntLit(_, pos)
        | Expr::FloatLit(_, pos)
        | Expr::StringLit(_, pos)
        | Expr::BoolLit(_, pos)
        | Expr::NoneLit(pos)
        | Expr::TypeProp { pos, .. }
        | Expr::Ident(_, pos) => stamp(pos, file),

        Expr::ArrayLit(items, pos) => {
            stamp(pos, file);
            for i in items {
                annotate_expr(i, file);
            }
        }

        Expr::MapLit(entries, pos) => {
            stamp(pos, file);
            for (k, v) in entries {
                annotate_expr(k, file);
                annotate_expr(v, file);
            }
        }

        Expr::Binary { left, right, pos, .. } => {
            stamp(pos, file);
            annotate_expr(left, file);
            annotate_expr(right, file);
        }

        Expr::Unary { expr, pos, .. } => {
            stamp(pos, file);
            annotate_expr(expr, file);
        }

        Expr::IncDec { target, pos, .. } => {
            stamp(pos, file);
            annotate_expr(target, file);
        }

        Expr::Call { callee, args, pos } => {
            stamp(pos, file);
            annotate_expr(callee, file);
            for a in args {
                annotate_expr(a, file);
            }
        }

        Expr::Index { array, index, pos } => {
            stamp(pos, file);
            annotate_expr(array, file);
            annotate_expr(index, file);
        }

        Expr::StructLit { fields, pos, .. } => {
            stamp(pos, file);
            for (_, v) in fields {
                annotate_expr(v, file);
            }
        }

        Expr::Field { object, pos, .. } => {
            stamp(pos, file);
            annotate_expr(object, file);
        }

        Expr::IfExpr { cond, then_branch, else_branch, pos } => {
            stamp(pos, file);
            annotate_expr(cond, file);
            annotate_block(then_branch, file);
            annotate_block(else_branch, file);
        }

        Expr::Lambda { params, return_type, body, pos } => {
            stamp(pos, file);
            annotate_params(params, file);
            if let Some(rt) = return_type {
                annotate_type(rt, file);
            }
            annotate_block(body, file);
        }
    }
}
