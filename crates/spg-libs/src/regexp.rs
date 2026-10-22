//! `regex`: regular expressions (`install #regex`). the `regex` crate: linear time, no
//! catastrophic backtracking, so a hostile pattern can be slow to compile but can not hang a
//! match. a bad pattern is an error the program can `catch`, not a crash.

use crate::{CallCtx, Kind, LibError, LibValue, NativeFn, NativeModule};

pub static MODULE: NativeModule = NativeModule {
    kind: Kind::Install,
    name: "regex",
    doc: "regular expressions (install #regex)",
    structs: &[],
    functions: &[
        NativeFn { name: "matches", sig: "(string, string) -> bool", doc: "true if the pattern appears anywhere in the text", call: matches },
        NativeFn { name: "findAll", sig: "(string, string) -> array<string>", doc: "every piece of the text the pattern matches, in order", call: find_all },
        NativeFn { name: "split", sig: "(string, string) -> array<string>", doc: "split the text wherever the pattern matches", call: split },
        NativeFn { name: "replace", sig: "(string, string, string) -> string", doc: "replace every match. the replacement can use $1, $2... for the groups", call: replace },
    ],
};

/// argument `i` as text (the typechecker already guaranteed it is a string)
fn s(args: &[LibValue], i: usize) -> &str {
    match args.get(i) {
        Some(LibValue::Str(text)) => text.as_str(),
        _ => "",
    }
}

/// the compiled pattern, or an error that says what is wrong with it
fn compile(function: &'static str, pattern: &str) -> Result<regex::Regex, LibError> {
    regex::Regex::new(pattern).map_err(|e| LibError::new(function, format!("invalid pattern: {}", first_line(&e.to_string()))))
}

/// the regex crate's message is several lines (the pattern, a caret), and the cause is on the
/// line that starts with `error:` (`unclosed group`). use that; fall back to the first line
fn first_line(text: &str) -> &str {
    text.lines()
        .find_map(|l| l.trim().strip_prefix("error:").map(str::trim))
        .or_else(|| text.lines().find(|l| !l.trim().is_empty()).map(str::trim))
        .unwrap_or(text)
}

fn matches(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    let re = compile("regex.matches", s(&a, 1))?;
    Ok(LibValue::Bool(re.is_match(s(&a, 0))))
}

fn find_all(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    let re = compile("regex.findAll", s(&a, 1))?;
    Ok(LibValue::Array(re.find_iter(s(&a, 0)).map(|m| LibValue::Str(m.as_str().to_string())).collect()))
}

fn split(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    let re = compile("regex.split", s(&a, 1))?;
    Ok(LibValue::Array(re.split(s(&a, 0)).map(|piece| LibValue::Str(piece.to_string())).collect()))
}

fn replace(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    let re = compile("regex.replace", s(&a, 1))?;
    Ok(LibValue::Str(re.replace_all(s(&a, 0), s(&a, 2)).into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(f: crate::LibFn, args: Vec<&str>) -> Result<LibValue, LibError> {
        f(&CallCtx { caller_file: None, caller_line: 0 }, args.into_iter().map(|a| LibValue::Str(a.to_string())).collect())
    }

    #[test]
    fn matches_and_find_all() {
        assert_eq!(call(matches, vec!["ana 42 bia 7", r"\d+"]).unwrap(), LibValue::Bool(true));
        assert_eq!(call(matches, vec!["abc", r"\d"]).unwrap(), LibValue::Bool(false));
        assert_eq!(
            call(find_all, vec!["ana 42 bia 7", r"\d+"]).unwrap(),
            LibValue::Array(vec![LibValue::Str("42".into()), LibValue::Str("7".into())])
        );
    }

    #[test]
    fn replace_uses_groups() {
        assert_eq!(
            call(replace, vec!["2026-10-21", r"(\d+)-(\d+)-(\d+)", "$3/$2/$1"]).unwrap(),
            LibValue::Str("21/10/2026".into())
        );
    }

    #[test]
    fn a_bad_pattern_is_an_error_with_a_short_reason() {
        let err = call(matches, vec!["x", "(abc"]).unwrap_err();
        assert_eq!(err.function, "regex.matches");
        assert!(err.reason.starts_with("invalid pattern:"), "{}", err.reason);
        assert!(err.reason.contains("unclosed group"), "{}", err.reason);
        assert!(!err.reason.contains('\n'));
    }
}
