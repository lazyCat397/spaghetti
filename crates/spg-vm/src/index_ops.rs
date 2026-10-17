//! out-of-line bodies of the fused array-index opcodes (`IndexGetDupLocals`,
//! `IndexAddConstLocals`, `Index<Add|Sub|Mul>[Float]LocalLocals`,
//! `IndexSetLocals*`). each one is the exact behavior of the instruction
//! sequence it replaces, check order included, so the optimized
//! and `--no-opt` programs fail the same way. yes, I keep saying this.
//!
//! they're `#[inline(never)]` on purpose. `Vm::step` is one huge `match`, and
//! pasting these bodies into it made plain arithmetic loops 3-8% slower
//! (measured: `loop_arith` 852 -> 922 ms). even though no index op runs in
//! them. a bigger function = worse register allocation for every arm. a
//! call costs a couple of ns. worth it. keeps `step` small. rust compiler, you're welcome.

use crate::error::RuntimeError;
use crate::value::Value;
use crate::vm::local_int;

pub(crate) fn local_float(locals: &[Value], slot: usize) -> Result<f64, RuntimeError> {
    match &locals[slot] {
        Value::Float(n) => Ok(*n),
        other => Err(RuntimeError::TypeMismatch { expected: "float", found: other.type_name() }),
    }
}

fn not_array(other: &Value) -> RuntimeError {
    RuntimeError::TypeMismatch { expected: "array", found: other.type_name() }
}

fn out_of_bounds(index: i64, len: usize) -> RuntimeError {
    RuntimeError::IndexOutOfBounds { index, len }
}

/// `GetLocal(arr); GetLocal(i); Dup2; IndexGet`: pushes the array, the index and
/// the current element. three things for the price of one.
#[inline(never)]
pub(crate) fn get_dup_locals(locals: &[Value], stack: &mut Vec<Value>, arr: usize, i: usize) -> Result<(), RuntimeError> {
    let index = local_int(locals, i)?;
    let (array, elem) = match &locals[arr] {
        Value::Array(a) => {
            let items = a.borrow();
            if index < 0 || index as usize >= items.len() {
                return Err(out_of_bounds(index, items.len()));
            }
            (a.clone(), items[index as usize].clone())
        }
        other => return Err(not_array(other)),
    };
    stack.push(Value::Array(array));
    stack.push(Value::Int(index));
    stack.push(elem);
    Ok(())
}

/// `IndexGetDupLocals; AddConst(k); IndexSet`: `arr[idx] += k`, in place.
#[inline(never)]
pub(crate) fn add_const_locals(locals: &[Value], arr: usize, idx: usize, k: i64) -> Result<(), RuntimeError> {
    let index = local_int(locals, idx)?;
    match &locals[arr] {
        Value::Array(a) => {
            let mut items = a.borrow_mut();
            if index < 0 || index as usize >= items.len() {
                return Err(out_of_bounds(index, items.len()));
            }
            match &mut items[index as usize] {
                Value::Int(n) => *n = crate::arith::add(*n, k)?, // same `x + k` as AddConst
                other => return Err(RuntimeError::TypeMismatch { expected: "int", found: other.type_name() }),
            }
            Ok(())
        }
        other => Err(not_array(other)),
    }
}

/// `IndexGetDupLocals; GetLocal(x); <op>; IndexSet`: `arr[idx] OP= x`, in place.
/// order of checks, same as the sequence: operand type, index type,
/// array type, bounds, element type. the bounds check is the one that hurts.
macro_rules! op_locals {
    ($name:ident, $local:ident, $variant:ident, $expected:literal, $f:expr) => {
        #[inline(never)]
        pub(crate) fn $name(locals: &[Value], arr: usize, idx: usize, x: usize) -> Result<(), RuntimeError> {
            let v = $local(locals, x)?;
            let index = local_int(locals, idx)?;
            match &locals[arr] {
                Value::Array(a) => {
                    let mut items = a.borrow_mut();
                    if index < 0 || index as usize >= items.len() {
                        return Err(out_of_bounds(index, items.len()));
                    }
                    match &mut items[index as usize] {
                        Value::$variant(n) => *n = $f(*n, v)?,
                        other => return Err(RuntimeError::TypeMismatch { expected: $expected, found: other.type_name() }),
                    }
                    Ok(())
                }
                other => Err(not_array(other)),
            }
        }
    };
}
op_locals!(int_add, local_int, Int, "int", crate::arith::add);
op_locals!(int_sub, local_int, Int, "int", crate::arith::sub);
op_locals!(int_mul, local_int, Int, "int", crate::arith::mul);
op_locals!(float_add, local_float, Float, "float", |a: f64, b: f64| -> Result<f64, RuntimeError> { Ok(a + b) });
op_locals!(float_sub, local_float, Float, "float", |a: f64, b: f64| -> Result<f64, RuntimeError> { Ok(a - b) });
op_locals!(float_mul, local_float, Float, "float", |a: f64, b: f64| -> Result<f64, RuntimeError> { Ok(a * b) });

/// `GetLocal(arr); GetLocal(idx); GetLocal(v); IndexSet`: `arr[idx] = v`.
#[inline(never)]
pub(crate) fn set_locals_local(locals: &[Value], arr: usize, idx: usize, v: usize) -> Result<(), RuntimeError> {
    store(locals, arr, idx, locals[v].clone())
}

/// `GetLocal(arr); GetLocal(idx); Const(c); IndexSet`: `arr[idx] = <const>`.
#[inline(never)]
pub(crate) fn set_locals_const(locals: &[Value], constants: &[Value], arr: usize, idx: usize, c: usize) -> Result<(), RuntimeError> {
    store(locals, arr, idx, constants[c].clone())
}

#[inline(always)]
fn store(locals: &[Value], arr: usize, idx: usize, value: Value) -> Result<(), RuntimeError> {
    // same order as `IndexSet`: index type, then array type, then bounds. don't reorder it.
    let index = local_int(locals, idx)?;
    match &locals[arr] {
        Value::Array(a) => {
            let mut items = a.borrow_mut();
            if index < 0 || index as usize >= items.len() {
                return Err(out_of_bounds(index, items.len()));
            }
            items[index as usize] = value;
            Ok(())
        }
        other => Err(not_array(other)),
    }
}

/// `GetLocal(idx); GetLocal(v); IndexSet` with the array already on the stack
/// (`m[i][j] = v`). nested arrays: the usual pain, now faster.
#[inline(never)]
pub(crate) fn set_local_local(locals: &[Value], stack: &mut Vec<Value>, idx: usize, v: usize) -> Result<(), RuntimeError> {
    let index = local_int(locals, idx)?;
    let value = locals[v].clone();
    match stack.pop() {
        Some(Value::Array(array)) => {
            let mut items = array.borrow_mut();
            if index < 0 || index as usize >= items.len() {
                return Err(out_of_bounds(index, items.len()));
            }
            items[index as usize] = value;
            Ok(())
        }
        Some(other) => Err(not_array(&other)),
        None => Err(RuntimeError::StackUnderflow),
    }
}
