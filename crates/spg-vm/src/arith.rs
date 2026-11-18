//! Integer arithmetic with ONE defined behaviour on overflow. (pick one, we picked.)
//!
//! Without `#> strict`, an int that doesn't fit in 64 bits wraps around (two's
//! complement). with `#> strict`, same case = runtime error. the check is one
//! well-predicted branch on the hot path. the flag only gets read once an
//! overflow has actually happened. so it's basically free. basically.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::chunk::SizedOp;
use crate::error::RuntimeError;

static STRICT: AtomicBool = AtomicBool::new(false);

/// `#> strict`: int overflow becomes a runtime error instead of wrapping. you asked for it.
pub fn set_strict(on: bool) {
    STRICT.store(on, Ordering::Relaxed);
}

#[cold]
#[inline(never)]
fn overflow(wrapped: i64, op: &'static str) -> Result<i64, RuntimeError> {
    if STRICT.load(Ordering::Relaxed) {
        Err(RuntimeError::IntegerOverflow { op })
    } else {
        Ok(wrapped)
    }
}

#[inline(always)]
pub(crate) fn add(a: i64, b: i64) -> Result<i64, RuntimeError> {
    match a.checked_add(b) {
        Some(v) => Ok(v),
        None => overflow(a.wrapping_add(b), "addition"),
    }
}

#[inline(always)]
pub(crate) fn sub(a: i64, b: i64) -> Result<i64, RuntimeError> {
    match a.checked_sub(b) {
        Some(v) => Ok(v),
        None => overflow(a.wrapping_sub(b), "subtraction"),
    }
}

#[inline(always)]
pub(crate) fn mul(a: i64, b: i64) -> Result<i64, RuntimeError> {
    match a.checked_mul(b) {
        Some(v) => Ok(v),
        None => overflow(a.wrapping_mul(b), "multiplication"),
    }
}

/// `MIN / -1` is the only overflowing division. the one that bites.
#[inline(always)]
pub(crate) fn div(a: i64, b: i64) -> Result<i64, RuntimeError> {
    if b == 0 {
        return Err(RuntimeError::DivisionByZero);
    }
    match a.checked_div(b) {
        Some(v) => Ok(v),
        None => overflow(a.wrapping_div(b), "division"),
    }
}

/// `MIN % -1` is mathematically 0, so it's not an overflow. Rust panics on it anyway, so we special case it. ugh.
#[inline(always)]
pub(crate) fn rem(a: i64, b: i64) -> Result<i64, RuntimeError> {
    if b == 0 {
        Err(RuntimeError::DivisionByZero)
    } else {
        Ok(a.wrapping_rem(b))
    }
}

#[inline(always)]
pub(crate) fn neg(a: i64) -> Result<i64, RuntimeError> {
    match a.checked_neg() {
        Some(v) => Ok(v),
        None => overflow(a.wrapping_neg(), "negation"),
    }
}

#[inline(always)]
pub(crate) fn abs(a: i64) -> Result<i64, RuntimeError> {
    match a.checked_abs() {
        Some(v) => Ok(v),
        None => overflow(a.wrapping_abs(), "abs"),
    }
}

/// a negative exponent is a valid spg program, so it gets clamped to 0. an
/// exponent past `u32::MAX` gets clamped too (any |base| >= 2 overflows way earlier)
#[inline(always)]
pub(crate) fn pow(a: i64, b: i64) -> Result<i64, RuntimeError> {
    let e = b.max(0).min(u32::MAX as i64) as u32;
    match a.checked_pow(e) {
        Some(v) => Ok(v),
        None => overflow(a.wrapping_pow(e), "power"),
    }
}

/// arithmetic on a sized int. the operands are already in `lo..=hi`. the result is computed in i64
/// (checked), then checked against the range. `wraps` types (`i8`, `u8`) wrap into the range, like
/// `int` wraps at 64 bits; `#> strict` makes any such case an error. a wrap from i64 is exact here,
/// because the range size (256) divides 2^64. `u64` (`wraps` false) never wraps: any result outside
/// 0..=i64::MAX, including a 64-bit overflow, is an error.
pub(crate) fn sized(op: SizedOp, a: i64, b: i64, lo: i64, hi: i64, wraps: bool, ty: &'static str) -> Result<i64, RuntimeError> {
    let (name, wide, wrapped) = match op {
        SizedOp::Add => ("addition", a.checked_add(b), a.wrapping_add(b)),
        SizedOp::Sub => ("subtraction", a.checked_sub(b), a.wrapping_sub(b)),
        SizedOp::Mul => ("multiplication", a.checked_mul(b), a.wrapping_mul(b)),
        SizedOp::Div => {
            if b == 0 {
                return Err(RuntimeError::DivisionByZero);
            }
            ("division", a.checked_div(b), a.wrapping_div(b))
        }
        SizedOp::Mod => {
            if b == 0 {
                return Err(RuntimeError::DivisionByZero);
            }
            ("remainder", a.checked_rem(b), a.wrapping_rem(b))
        }
        // same clamp as `pow` for int: a negative exponent is valid and gives 1
        SizedOp::Pow => {
            let e = b.max(0).min(u32::MAX as i64) as u32;
            ("power", a.checked_pow(e), a.wrapping_pow(e))
        }
    };
    match wide {
        Some(v) if (lo..=hi).contains(&v) => Ok(v),
        // out of range, or the i64 itself overflowed (then `wrapped` is the 64-bit wrap)
        _ => {
            if !wraps || STRICT.load(Ordering::Relaxed) {
                return Err(RuntimeError::SizedOutOfRange { ty, op: name, lo, hi, wraps });
            }
            Ok(wrap_into(wrapped, lo, hi))
        }
    }
}

/// arithmetic on a u64. the bits are the u64 (a negative i64 is a large u64). the result wraps at 2^64,
/// like `int` wraps at 2^64, and `#> strict` makes a result that leaves u64 an error. division by zero
/// is an error, as for the other types.
pub(crate) fn u64_arith(op: SizedOp, a: i64, b: i64) -> Result<i64, RuntimeError> {
    let (x, y) = (a as u64, b as u64);
    let (r, overflow, name) = match op {
        SizedOp::Add => {
            let (r, o) = x.overflowing_add(y);
            (r, o, "addition")
        }
        SizedOp::Sub => {
            let (r, o) = x.overflowing_sub(y);
            (r, o, "subtraction")
        }
        SizedOp::Mul => {
            let (r, o) = x.overflowing_mul(y);
            (r, o, "multiplication")
        }
        SizedOp::Div => {
            if y == 0 {
                return Err(RuntimeError::DivisionByZero);
            }
            (x / y, false, "division")
        }
        SizedOp::Mod => {
            if y == 0 {
                return Err(RuntimeError::DivisionByZero);
            }
            (x % y, false, "remainder")
        }
        // a negative exponent cannot come from a u64; a huge one overflows anyway
        SizedOp::Pow => {
            let e = y.min(u32::MAX as u64) as u32;
            let (r, o) = x.overflowing_pow(e);
            (r, o, "power")
        }
    };
    if overflow && STRICT.load(Ordering::Relaxed) {
        return Err(RuntimeError::UnsignedOverflow { op: name });
    }
    Ok(r as i64)
}

/// `@into.T(v)` / `@into.Tw(v)`: an int into a sized type. in range: the value itself. out of range:
/// `wraps` (the `w` names) wraps it, the others are an error that names the `w` version.
pub(crate) fn into_sized(value: i64, lo: i64, hi: i64, wraps: bool, ty: &'static str, suggest_wrap: bool) -> Result<i64, RuntimeError> {
    // a u64 target: its bits are the value (a wrap of 2^64 is the identity on bits); an int must be >= 0
    if ty == "u64" {
        return if wraps || value >= 0 {
            Ok(value)
        } else {
            Err(RuntimeError::IntoOutOfRange { value, ty, lo, hi, suggest_wrap })
        };
    }
    if (lo..=hi).contains(&value) {
        Ok(value)
    } else if wraps {
        Ok(wrap_into(value, lo, hi))
    } else {
        Err(RuntimeError::IntoOutOfRange { value, ty, lo, hi, suggest_wrap })
    }
}

/// `v` moved into `lo..=hi` modulo the range size (`hi - lo + 1`). `i8` and `u8` only.
fn wrap_into(v: i64, lo: i64, hi: i64) -> i64 {
    let width = (hi as i128) - (lo as i128) + 1;
    (((v as i128) - (lo as i128)).rem_euclid(width) + lo as i128) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    // one test on purpose: the strict flag is global, so two tests flipping it
    // could run in parallel and wreck each other. global state is a menace.
    #[test]
    fn wraps_by_default_and_errors_in_strict() {
        set_strict(false);
        assert_eq!(add(i64::MAX, 1).unwrap(), i64::MIN);
        assert_eq!(sub(i64::MIN, 1).unwrap(), i64::MAX);
        assert_eq!(mul(i64::MAX, 2).unwrap(), -2);
        assert_eq!(neg(i64::MIN).unwrap(), i64::MIN);
        assert_eq!(div(i64::MIN, -1).unwrap(), i64::MIN);
        assert_eq!(rem(i64::MIN, -1).unwrap(), 0);
        assert_eq!(pow(2, 64).unwrap(), 0);
        assert!(matches!(div(1, 0), Err(RuntimeError::DivisionByZero)));

        set_strict(true);
        assert!(matches!(add(i64::MAX, 1), Err(RuntimeError::IntegerOverflow { op: "addition" })));
        assert!(matches!(mul(i64::MAX, 2), Err(RuntimeError::IntegerOverflow { .. })));
        assert!(matches!(pow(2, 64), Err(RuntimeError::IntegerOverflow { op: "power" })));
        assert!(matches!(abs(i64::MIN), Err(RuntimeError::IntegerOverflow { .. })));
        assert_eq!(add(1, 2).unwrap(), 3);
        assert_eq!(rem(i64::MIN, -1).unwrap(), 0);
        set_strict(false);
    }
}
