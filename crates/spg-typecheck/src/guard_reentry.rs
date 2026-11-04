//! Compile-time check: a struct's guard must not reach a write to that same struct.
//!
//! The runtime already refuses such a write (see `@guard.enter`). This pass only gives
//! the error before the program runs, with the whole path. It follows what it can see:
//!
//! - a direct call to a named function or method (`reset(me)`, `me.__guard()`);
//! - a guarded write `@guard.enter("X")` leads to the guard of `X`, `X.__guard`.
//!
//! It does not follow calls through function values (`CallValue`), lambda bodies
//! or native libraries. Those stay with the runtime check.
use std::collections::{HashMap, HashSet};

use crate::builtins::BuiltinId;
use crate::error::TypeError;
use crate::typed_ast::{RangeStep, TypedBlock, TypedExpr, TypedStmt};
use spg_parser::ast::Pos;

/// what a piece of code can lead to, as far as this pass can see
#[derive(Debug, PartialEq)]
enum Edge {
    /// a call to a named function or method
    Call(String),
    /// a guarded write to a struct type: runs that type's guard
    Write(String),
}

/// `positions` maps each `X.__guard` name to the source position of its struct.
/// returns one error per guard that can reach a write to its own struct
pub(crate) fn check(stmts: &[TypedStmt], positions: &HashMap<String, Pos>) -> Vec<TypeError> {
    let mut bodies: HashMap<&str, &TypedBlock> = HashMap::new();
    for s in stmts {
        if let TypedStmt::FunctionDecl { name, body, .. } = s {
            bodies.insert(name.as_str(), body);
        }
    }
    let mut errors = Vec::new();
    let mut guards: Vec<&str> = bodies.keys().copied().filter(|n| n.ends_with(".__guard")).collect();
    guards.sort(); // a stable order for the errors
    for guard in guards {
        let struct_name = &guard[..guard.len() - ".__guard".len()];
        let mut path = Vec::new();
        let mut visited = HashSet::new();
        if reaches_write(guard, struct_name, &bodies, &mut visited, &mut path) {
            let pos = positions.get(guard).cloned().unwrap_or_else(|| Pos { line: 0, col: 0, file: None });
            errors.push(TypeError::GuardReentry {
                struct_name: struct_name.to_string(),
                path: path.iter().map(|n| display(n)).collect(),
                pos,
            });
        }
    }
    errors
}

/// is there a path from `node` to a guarded write of `target`? fills `path` with the
/// functions on it (the first one is `node`)
fn reaches_write<'a>(
    node: &'a str,
    target: &str,
    bodies: &HashMap<&'a str, &'a TypedBlock>,
    visited: &mut HashSet<&'a str>,
    path: &mut Vec<String>,
) -> bool {
    if !visited.insert(node) {
        return false;
    }
    let Some(body) = bodies.get(node) else { return false };
    let mut edges = Vec::new();
    block_edges(body, &mut edges);
    path.push(node.to_string());
    for edge in edges {
        match edge {
            Edge::Write(t) if t == target => return true,
            Edge::Write(t) => {
                let next = format!("{}.__guard", t);
                if let Some((key, _)) = bodies.get_key_value(next.as_str()) {
                    if reaches_write(*key, target, bodies, visited, path) {
                        return true;
                    }
                }
            }
            Edge::Call(f) => {
                if let Some((key, _)) = bodies.get_key_value(f.as_str()) {
                    if reaches_write(*key, target, bodies, visited, path) {
                        return true;
                    }
                }
            }
        }
    }
    path.pop();
    false
}

/// `Point.__guard` reads as `guard of Point`, other names stay as they are
fn display(name: &str) -> String {
    match name.strip_suffix(".__guard") {
        Some(s) => format!("guard of {}", s),
        None => name.to_string(),
    }
}

fn block_edges(b: &TypedBlock, out: &mut Vec<Edge>) {
    for s in &b.stmts {
        stmt_edges(s, out);
    }
}

fn stmt_edges(s: &TypedStmt, out: &mut Vec<Edge>) {
    match s {
        TypedStmt::Let { value, .. } | TypedStmt::Const { value, .. } => expr_edges(value, out),
        TypedStmt::Assign { value, .. } => expr_edges(value, out),
        TypedStmt::IndexAssign { array, index, value } | TypedStmt::IndexCompoundAssign { array, index, value, .. } => {
            expr_edges(array, out);
            expr_edges(index, out);
            expr_edges(value, out);
        }
        TypedStmt::FieldAssign { object, value, .. } | TypedStmt::FieldCompoundAssign { object, value, .. } => {
            expr_edges(object, out);
            expr_edges(value, out);
        }
        TypedStmt::IfStmt { cond, then_branch, else_branch } => {
            expr_edges(cond, out);
            block_edges(then_branch, out);
            if let Some(b) = else_branch {
                block_edges(b, out);
            }
        }
        TypedStmt::Block { body } | TypedStmt::CompileVm { body } | TypedStmt::CompileJit { body } => block_edges(body, out),
        TypedStmt::While { cond, body } | TypedStmt::DoWhile { body, cond } => {
            expr_edges(cond, out);
            block_edges(body, out);
        }
        TypedStmt::ForIn { iterable, body, map_source, .. } => {
            expr_edges(iterable, out);
            if let Some(m) = map_source {
                expr_edges(m, out);
            }
            block_edges(body, out);
        }
        TypedStmt::ForRange { start, end, step, body, .. } => {
            expr_edges(start, out);
            expr_edges(end, out);
            if let RangeStep::Dynamic(e) = step {
                expr_edges(e, out);
            }
            block_edges(body, out);
        }
        TypedStmt::Return { value } => {
            if let Some(v) = value {
                expr_edges(v, out);
            }
        }
        TypedStmt::Throw { value } => expr_edges(value, out),
        TypedStmt::Try { try_block, catch_block, .. } => {
            block_edges(try_block, out);
            block_edges(catch_block, out);
        }
        TypedStmt::ExprStmt(e) => expr_edges(e, out),
        // declarations, breaks and imports run no code here
        _ => {}
    }
}

fn expr_edges(e: &TypedExpr, out: &mut Vec<Edge>) {
    match e {
        TypedExpr::Call { callee, args, builtin, .. } => {
            match builtin {
                Some(BuiltinId::GuardEnter) => {
                    if let Some(TypedExpr::StringLit(t)) = args.first() {
                        out.push(Edge::Write(t.clone()));
                    }
                }
                Some(_) => {}
                None => {
                    if let TypedExpr::Ident { name, .. } = &**callee {
                        out.push(Edge::Call(name.clone()));
                    }
                }
            }
            args.iter().for_each(|a| expr_edges(a, out));
        }
        // a call through a value: the target is not known here, the runtime check covers it
        TypedExpr::CallValue { callee, args, .. } => {
            expr_edges(callee, out);
            args.iter().for_each(|a| expr_edges(a, out));
        }
        TypedExpr::ArrayLit { items, .. } => items.iter().for_each(|i| expr_edges(i, out)),
        TypedExpr::MapLit { entries, .. } => entries.iter().for_each(|(k, v)| {
            expr_edges(k, out);
            expr_edges(v, out);
        }),
        TypedExpr::Coalesce { left, right, .. } => {
            expr_edges(left, out);
            expr_edges(right, out);
        }
        TypedExpr::Binary { left, right, .. } => {
            expr_edges(left, out);
            expr_edges(right, out);
        }
        TypedExpr::Unary { expr, .. } | TypedExpr::AsInt(expr) => expr_edges(expr, out),
        TypedExpr::EnumValue { operand, .. } => expr_edges(operand, out),
        TypedExpr::ArrayFn { array, func, init, .. } => {
            expr_edges(array, out);
            expr_edges(func, out);
            if let Some(i) = init {
                expr_edges(i, out);
            }
        }
        TypedExpr::MapFn { map, func, .. } => {
            expr_edges(map, out);
            expr_edges(func, out);
        }
        TypedExpr::IncDec { target, .. } => expr_edges(target, out),
        TypedExpr::Index { array, index, .. } => {
            expr_edges(array, out);
            expr_edges(index, out);
        }
        TypedExpr::StructLit { fields, .. } => fields.iter().for_each(|f| expr_edges(f, out)),
        TypedExpr::Field { object, .. } => expr_edges(object, out),
        TypedExpr::IfExpr { cond, then_branch, else_branch, .. } => {
            expr_edges(cond, out);
            block_edges(then_branch, out);
            block_edges(else_branch, out);
        }
        // literals, names, lambdas (their body runs only when called through a value)
        _ => {}
    }
}
