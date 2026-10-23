//! out-of-line bodies of the fused struct-field opcodes (`FieldAddConst`,
//! `Field{Int,Float}{Add,Sub,Mul}`, `FieldSetLocal*`). each one does the exact
//! same thing as the instruction sequence it replaces, same checks in
//! the same order, so optimized and `--no-opt` programs fail the same way. consistency.
//! out of line for the same reason as `index_ops`: keep `Vm::step` small.

use crate::error::RuntimeError;
use crate::index_ops::local_float;
use crate::value::Value;
use crate::vm::local_int;

fn not_struct(other: &Value) -> RuntimeError {
    RuntimeError::TypeMismatch { expected: "struct", found: other.type_name() }
}

/// `FieldGetDup; AddConst(k); FieldSet`: `s.f += k`, in place. boring and fast.
#[inline(never)]
pub(crate) fn add_const(locals: &[Value], local: usize, field: usize, k: i64) -> Result<(), RuntimeError> {
    match &locals[local] {
        Value::Struct(s) => {
            crate::frozen::check_struct(s)?;
            let mut b = s.borrow_mut();
            match &mut b.1[field] {
                Value::Int(n) => {
                    *n = crate::arith::add(*n, k)?; // same `x + k` as AddConst
                    Ok(())
                }
                other => Err(RuntimeError::TypeMismatch { expected: "int", found: other.type_name() }),
            }
        }
        other => Err(not_struct(other)),
    }
}

/// `FieldGetDup; GetLocal(x); <op>; FieldSet`: `s.f OP= x`, in place. the
/// operand's type gets checked first, like the arithmetic op would. order matters.
macro_rules! op_locals {
    ($name:ident, $local:ident, $variant:ident, $expected:literal, $f:expr) => {
        #[inline(never)]
        pub(crate) fn $name(locals: &[Value], local: usize, field: usize, x: usize) -> Result<(), RuntimeError> {
            let v = $local(locals, x)?;
            match &locals[local] {
                Value::Struct(s) => {
                    crate::frozen::check_struct(s)?;
                    let mut b = s.borrow_mut();
                    match &mut b.1[field] {
                        Value::$variant(n) => {
                            *n = $f(*n, v)?;
                            Ok(())
                        }
                        other => Err(RuntimeError::TypeMismatch { expected: $expected, found: other.type_name() }),
                    }
                }
                other => Err(not_struct(other)),
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

#[inline(always)]
fn store(locals: &[Value], local: usize, field: usize, value: Value) -> Result<(), RuntimeError> {
    match &locals[local] {
        Value::Struct(s) => {
            crate::frozen::check_struct(s)?;
            s.borrow_mut().1[field] = value;
            Ok(())
        }
        other => Err(not_struct(other)),
    }
}

/// `GetLocal(s); GetLocal(v); FieldSet`: `s.f = v`. nothing to see here.
#[inline(never)]
pub(crate) fn set_local_local(locals: &[Value], local: usize, field: usize, v: usize) -> Result<(), RuntimeError> {
    store(locals, local, field, locals[v].clone())
}

/// `GetLocal(s); Const(c); FieldSet`: `s.f = <const>`. same, but cheaper, we hope.
#[inline(never)]
pub(crate) fn set_local_const(locals: &[Value], constants: &[Value], local: usize, field: usize, c: usize) -> Result<(), RuntimeError> {
    store(locals, local, field, constants[c].clone())
}
