//! `@char`: single characters. core (always there, written with `@`). a character here is a
//! Unicode code point (what `len` counts), not a byte. the `is*` tests look at the first
//! character only, and are false for empty text.

use crate::{CallCtx, Kind, LibError, LibValue, NativeFn, NativeModule};

pub static MODULE: NativeModule = NativeModule {
    kind: Kind::Core,
    name: "char",
    doc: "single characters: codes and kind tests",
    structs: &[],
    functions: &[
        NativeFn { name: "code", sig: "(string) -> int", doc: "the code point of the first character (error on empty text)", call: code },
        NativeFn { name: "fromCode", sig: "(int) -> string", doc: "the character for a code point (error if there is none)", call: from },
        NativeFn { name: "isDigit", sig: "(string) -> bool", doc: "true if the first character is a digit 0-9", call: is_digit },
        NativeFn { name: "isAlpha", sig: "(string) -> bool", doc: "true if the first character is a letter of any alphabet", call: is_alpha },
        NativeFn { name: "isSpace", sig: "(string) -> bool", doc: "true if the first character is whitespace", call: is_space },
        NativeFn { name: "isUpper", sig: "(string) -> bool", doc: "true if the first character is uppercase", call: is_upper },
        NativeFn { name: "isLower", sig: "(string) -> bool", doc: "true if the first character is lowercase", call: is_lower },
    ],
};

fn text(a: &[LibValue]) -> &str {
    match a.first() {
        Some(LibValue::Str(t)) => t.as_str(),
        _ => "",
    }
}

fn first(a: &[LibValue]) -> Option<char> {
    text(a).chars().next()
}

fn kind_of(a: &[LibValue], check: fn(char) -> bool) -> Result<LibValue, LibError> {
    Ok(LibValue::Bool(first(a).is_some_and(check)))
}

fn code(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    match first(&a) {
        Some(c) => Ok(LibValue::Int(c as i64)),
        None => Err(LibError::new("@char.code", "the text is empty, so it has no first character")),
    }
}

fn from(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    let n = match a.first() {
        Some(LibValue::Int(n)) => *n,
        _ => 0,
    };
    u32::try_from(n)
        .ok()
        .and_then(char::from_u32)
        .map(|c| LibValue::Str(c.to_string()))
        .ok_or_else(|| LibError::new("@char.from", format!("{} is not a character code", n)))
}

fn is_digit(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    kind_of(&a, |c| c.is_ascii_digit())
}

fn is_alpha(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    kind_of(&a, |c| c.is_alphabetic())
}

fn is_space(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    kind_of(&a, |c| c.is_whitespace())
}

fn is_upper(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    kind_of(&a, |c| c.is_uppercase())
}

fn is_lower(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    kind_of(&a, |c| c.is_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LibFn;

    fn call(f: LibFn, v: LibValue) -> Result<LibValue, LibError> {
        f(&CallCtx { caller_file: None, caller_line: 0 }, vec![v])
    }

    fn st(s: &str) -> LibValue {
        LibValue::Str(s.to_string())
    }

    #[test]
    fn code_and_from_round_trip() {
        assert_eq!(call(code, st("A")).unwrap(), LibValue::Int(65));
        assert_eq!(call(code, st("é!")).unwrap(), LibValue::Int(233)); // first character only
        assert_eq!(call(from, LibValue::Int(65)).unwrap(), st("A"));
        assert!(call(code, st("")).is_err());
        assert!(call(from, LibValue::Int(-1)).is_err());
        assert!(call(from, LibValue::Int(0xD800)).is_err()); // a surrogate is not a character
    }

    #[test]
    fn kind_tests_look_at_the_first_character_only() {
        assert_eq!(call(is_digit, st("7a")).unwrap(), LibValue::Bool(true));
        assert_eq!(call(is_digit, st("a7")).unwrap(), LibValue::Bool(false));
        assert_eq!(call(is_digit, st("")).unwrap(), LibValue::Bool(false));
        assert_eq!(call(is_alpha, st("é")).unwrap(), LibValue::Bool(true));
        assert_eq!(call(is_space, st(" x")).unwrap(), LibValue::Bool(true));
        assert_eq!(call(is_upper, st("Ab")).unwrap(), LibValue::Bool(true));
        assert_eq!(call(is_lower, st("Ab")).unwrap(), LibValue::Bool(false));
    }
}
