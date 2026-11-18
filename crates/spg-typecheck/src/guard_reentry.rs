//! Compile-time check: a struct's guard must not reach a write to that same struct.
//!
//! The runtime already refuses such a write (see `@guard.enter`). This pass gives the
//! error before the program runs, with the chain of calls and the lines they are on.
//! It follows what it can see:
//!
//! - a direct call to a named function or method (`reset(me)`, `me.__guard()`);
//! - a call through a local name bound exactly once, with `let`, to a named function
//!   (`let f = reset`) or to a lambda (`let f = fn (...) { ... }`);
//! - a guarded write `@guard.enter("X")` leads to the guard of `X`, `X.__guard`.
//!
//! Anything else (a name bound twice or reassigned, a function value passed in, a native
//! library) is not followed. The runtime check covers those.
use std::collections::{HashMap, HashSet};

use crate::builtins::BuiltinId;
use crate::error::TypeError;
use crate::typed_ast::{RangeStep, TypedBlock, TypedExpr, TypedStmt};
use spg_parser::ast::Pos;

/// one thing the chain of calls goes through
#[derive(Clone)]
enum Step {
    /// a function or method, with the line it is called from (0 for the guard itself)
    Fn { name: String, line: usize },
    /// the guarded write itself, on the line where it happens
    Write { ty: String, line: usize },
}

/// what a piece of code leads to. `line` is the source line of the statement holding it
#[derive(Clone)]
enum Edge<'a> {
    /// a call to a named function or method
    Call(String, usize),
    /// a call through a local name, resolved once the whole body is seen
    Value(String, usize),
    /// a call of a lambda bound to a local name: its body runs here
    Lambda(&'a TypedBlock),
    /// a guarded write to a struct type: runs that type's guard
    Write(String, usize),
}

/// what a local name can stand for, when it is bound once with `let`
#[derive(Clone)]
enum Bound<'a> {
    Fn(String),
    Lambda(&'a TypedBlock),
}

/// one body's raw walk: its edges, how often each name is bound, and the `let`
/// bindings that could be followed
struct Walk<'a> {
    edges: Vec<Edge<'a>>,
    binders: HashMap<String, usize>,
    bound: HashMap<String, Bound<'a>>,
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
        let mut path: Vec<Step> = Vec::new();
        let mut visited = HashSet::new();
        if search_fn(guard, struct_name, &bodies, &mut visited, &mut path, 0) {
            let base = positions.get(guard).cloned().unwrap_or_else(|| Pos { line: 0, col: 0, file: None });
            // the chain starts at the first step after the guard: point at that line
            let first_line = match path.get(1) {
                Some(Step::Fn { line, .. }) | Some(Step::Write { line, .. }) => *line,
                None => base.line,
            };
            errors.push(TypeError::GuardReentry {
                struct_name: struct_name.to_string(),
                path: render(&path),
                pos: Pos { line: first_line, col: 1, file: base.file.clone() },
            });
        }
    }
    errors
}

/// searches a named function's body for a path to a guarded write of `target`.
/// `path` gets the steps on the way, the first being `node` itself
fn search_fn<'a>(
    node: &'a str,
    target: &str,
    bodies: &HashMap<&'a str, &'a TypedBlock>,
    visited: &mut HashSet<&'a str>,
    path: &mut Vec<Step>,
    line: usize,
) -> bool {
    if !visited.insert(node) {
        return false;
    }
    let Some(body) = bodies.get(node) else { return false };
    path.push(Step::Fn { name: node.to_string(), line });
    if search_edges(&block_edges(body), target, bodies, visited, path) {
        return true;
    }
    path.pop();
    false
}

/// searches a list of edges (from a function body or a lambda body) for the target write
fn search_edges<'a>(
    edges: &[Edge<'a>],
    target: &str,
    bodies: &HashMap<&'a str, &'a TypedBlock>,
    visited: &mut HashSet<&'a str>,
    path: &mut Vec<Step>,
) -> bool {
    for edge in edges {
        match edge {
            Edge::Write(t, l) if t == target => {
                path.push(Step::Write { ty: t.clone(), line: *l });
                return true;
            }
            Edge::Write(t, l) => {
                let next = format!("{}.__guard", t);
                if let Some((key, _)) = bodies.get_key_value(next.as_str()) {
                    if search_fn(*key, target, bodies, visited, path, *l) {
                        return true;
                    }
                }
            }
            Edge::Call(f, l) => {
                if let Some((key, _)) = bodies.get_key_value(f.as_str()) {
                    if search_fn(*key, target, bodies, visited, path, *l) {
                        return true;
                    }
                }
            }
            Edge::Lambda(body) => {
                // a lambda runs as part of the function that called it: its steps are on the path
                if search_edges(&block_edges(body), target, bodies, visited, path) {
                    return true;
                }
            }
            Edge::Value(..) => {}
        }
    }
    false
}

/// `X.__guard` reads as `guard of X`; a call reads as `name (line N)`, N being the line it
/// is called from; a write reads as `write to X (line N)`
fn render(path: &[Step]) -> Vec<String> {
    path.iter()
        .enumerate()
        .map(|(i, step)| match step {
            Step::Fn { name, line } => match name.strip_suffix(".__guard") {
                Some(s) if i == 0 => format!("guard of {}", s),
                _ => format!("{} (line {})", name, line),
            },
            Step::Write { ty, line } => format!("write to {} (line {})", ty, line),
        })
        .collect()
}

/// the edges of one body, with local names resolved. a name bound more than once,
/// or reassigned, has no fixed meaning and is dropped
fn block_edges(b: &TypedBlock) -> Vec<Edge<'_>> {
    let mut walk = Walk { edges: Vec::new(), binders: HashMap::new(), bound: HashMap::new() };
    walk_block(b, &mut walk);
    let mut out = Vec::new();
    for edge in walk.edges {
        match edge {
            Edge::Value(name, line) => {
                if walk.binders.get(&name) != Some(&1) {
                    continue;
                }
                match walk.bound.get(&name) {
                    Some(Bound::Fn(f)) => out.push(Edge::Call(f.clone(), line)),
                    Some(Bound::Lambda(body)) => out.push(Edge::Lambda(body)),
                    None => {}
                }
            }
            other => out.push(other),
        }
    }
    out
}

fn bind(walk: &mut Walk<'_>, name: &str) {
    *walk.binders.entry(name.to_string()).or_insert(0) += 1;
}

fn walk_block<'a>(b: &'a TypedBlock, walk: &mut Walk<'a>) {
    for (i, s) in b.stmts.iter().enumerate() {
        let line = b.locs.get(i).map(|l| l.line).unwrap_or(0);
        stmt_edges(s, line, walk);
    }
}

fn stmt_edges<'a>(s: &'a TypedStmt, line: usize, walk: &mut Walk<'a>) {
    match s {
        TypedStmt::Let { name, value, .. } | TypedStmt::Const { name, value, .. } => {
            bind(walk, name);
            match value {
                // `let f = reset`: the name stands for that function, if it is never rebound
                TypedExpr::Ident { name: f, .. } => {
                    walk.bound.insert(name.clone(), Bound::Fn(f.clone()));
                }
                TypedExpr::Lambda { body, .. } => {
                    walk.bound.insert(name.clone(), Bound::Lambda(body));
                }
                _ => {}
            }
            expr_edges(value, line, walk);
        }
        TypedStmt::Assign { name, value } => {
            bind(walk, name); // reassigned: no fixed meaning
            expr_edges(value, line, walk);
        }
        TypedStmt::IndexAssign { array, index, value } | TypedStmt::IndexCompoundAssign { array, index, value, .. } => {
            expr_edges(array, line, walk);
            expr_edges(index, line, walk);
            expr_edges(value, line, walk);
        }
        TypedStmt::FieldAssign { object, value, .. } | TypedStmt::FieldCompoundAssign { object, value, .. } => {
            expr_edges(object, line, walk);
            expr_edges(value, line, walk);
        }
        TypedStmt::IfStmt { cond, then_branch, else_branch } => {
            expr_edges(cond, line, walk);
            walk_block(then_branch, walk);
            if let Some(b) = else_branch {
                walk_block(b, walk);
            }
        }
        TypedStmt::Block { body } => walk_block(body, walk),
        TypedStmt::While { cond, body } | TypedStmt::DoWhile { body, cond } => {
            expr_edges(cond, line, walk);
            walk_block(body, walk);
        }
        TypedStmt::ForIn { var_name, index_var, iterable, body, map_source, .. } => {
            bind(walk, var_name);
            if let Some(i) = index_var {
                bind(walk, i);
            }
            expr_edges(iterable, line, walk);
            if let Some(m) = map_source {
                expr_edges(m, line, walk);
            }
            walk_block(body, walk);
        }
        TypedStmt::ForRange { var_name, start, end, step, body, .. } => {
            bind(walk, var_name);
            expr_edges(start, line, walk);
            expr_edges(end, line, walk);
            if let RangeStep::Dynamic(e) = step {
                expr_edges(e, line, walk);
            }
            walk_block(body, walk);
        }
        TypedStmt::Return { value } => {
            if let Some(v) = value {
                expr_edges(v, line, walk);
            }
        }
        TypedStmt::Throw { value } => expr_edges(value, line, walk),
        TypedStmt::Try { try_block, catch_var, catch_block, .. } => {
            walk_block(try_block, walk);
            bind(walk, catch_var);
            walk_block(catch_block, walk);
        }
        TypedStmt::ExprStmt(e) => expr_edges(e, line, walk),
        TypedStmt::Use { names, .. } => names.iter().for_each(|n| bind(walk, n)),
        // declarations and breaks run no code here
        _ => {}
    }
}

fn expr_edges<'a>(e: &'a TypedExpr, line: usize, walk: &mut Walk<'a>) {
    match e {
        TypedExpr::Call { callee, args, builtin, .. } => {
            match builtin {
                Some(BuiltinId::GuardEnter) => {
                    if let Some(TypedExpr::StringLit(t)) = args.first() {
                        walk.edges.push(Edge::Write(t.clone(), line));
                    }
                }
                Some(_) => {}
                None => {
                    if let TypedExpr::Ident { name, .. } = &**callee {
                        walk.edges.push(Edge::Call(name.clone(), line));
                    }
                }
            }
            args.iter().for_each(|a| expr_edges(a, line, walk));
        }
        // a call through a value: followed only when the name is simple (see block_edges)
        TypedExpr::CallValue { callee, args, .. } => {
            if let TypedExpr::Ident { name, .. } = &**callee {
                walk.edges.push(Edge::Value(name.clone(), line));
            } else {
                expr_edges(callee, line, walk);
            }
            args.iter().for_each(|a| expr_edges(a, line, walk));
        }
        // a lambda's parameters are bindings too; its body runs only when it is called
        TypedExpr::Lambda { params, .. } => params.iter().for_each(|p| bind(walk, &p.name)),
        TypedExpr::ArrayLit { items, .. } => items.iter().for_each(|i| expr_edges(i, line, walk)),
        TypedExpr::MapLit { entries, .. } => entries.iter().for_each(|(k, v)| {
            expr_edges(k, line, walk);
            expr_edges(v, line, walk);
        }),
        TypedExpr::Coalesce { left, right, .. } | TypedExpr::Binary { left, right, .. } => {
            expr_edges(left, line, walk);
            expr_edges(right, line, walk);
        }
        TypedExpr::Unary { expr, .. } | TypedExpr::AsInt(expr) => expr_edges(expr, line, walk),
        TypedExpr::EnumValue { operand, .. } => expr_edges(operand, line, walk),
        TypedExpr::ArrayFn { array, func, init, .. } => {
            expr_edges(array, line, walk);
            expr_edges(func, line, walk);
            if let Some(i) = init {
                expr_edges(i, line, walk);
            }
        }
        TypedExpr::MapFn { map, func, .. } => {
            expr_edges(map, line, walk);
            expr_edges(func, line, walk);
        }
        TypedExpr::IncDec { target, .. } => expr_edges(target, line, walk),
        TypedExpr::Index { array, index, .. } => {
            expr_edges(array, line, walk);
            expr_edges(index, line, walk);
        }
        TypedExpr::StructLit { fields, .. } => fields.iter().for_each(|f| expr_edges(f, line, walk)),
        TypedExpr::Field { object, .. } => expr_edges(object, line, walk),
        TypedExpr::IfExpr { cond, then_branch, else_branch, .. } => {
            expr_edges(cond, line, walk);
            walk_block(then_branch, walk);
            walk_block(else_branch, walk);
        }
        // literals and plain names
        _ => {}
    }
}
