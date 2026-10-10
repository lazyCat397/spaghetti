//! Integer arithmetic with ONE defined behaviour on overflow.
//!
//! Without `#> strict` an int that does not fit in 64 bits wraps around (two's
//! complement); with it, the same case is a runtime error. The check is one
//! well-predicted branch on the hot path; the flag is only read once an
//! overflow has actually happened.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::error::RuntimeError;

static STRICT: AtomicBool = AtomicBool::new(false);

/// `#> strict`: int overflow becomes a runtime error instead of wrapping
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

/// `MIN / -1` is the only overflowing division
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

/// `MIN % -1` is mathematically 0, so it is not an overflow
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

/// a negative exponent is a valid spg program, so it is clamped to 0; an
/// exponent past `u32::MAX` is clamped too (any |base| >= 2 overflows long before)
#[inline(always)]
pub(crate) fn pow(a: i64, b: i64) -> Result<i64, RuntimeError> {
    let e = b.max(0).min(u32::MAX as i64) as u32;
    match a.checked_pow(e) {
        Some(v) => Ok(v),
        None => overflow(a.wrapping_pow(e), "power"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // one test on purpose: the strict flag is global, so two tests flipping it
    // could run in parallel and trip each other
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
