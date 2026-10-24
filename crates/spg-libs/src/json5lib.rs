//! `json5`: read values from JSON5 text (`install #json5`). JSON5 is JSON as people write it by
//! hand: comments, trailing commas, unquoted names, single quotes, hex numbers. the text is
//! turned into strict JSON first, then read with the same parser and the same dotted paths as
//! `@json` (a number in the path picks an item of an array: `"servers.0.host"`). a path that is
//! not there gives the fallback the program passed. text that is not JSON5, or a value of
//! another type, is an error the program can catch. read only.

use crate::json::{self, Json};
use crate::{CallCtx, Kind, LibError, LibValue, NativeFn, NativeModule};

pub static MODULE: NativeModule = NativeModule {
    kind: Kind::Install,
    name: "json5",
    doc: "read values from JSON5 text by their dotted path (install #json5)",
    structs: &[],
    objects: &[],
    functions: &[
        NativeFn { name: "has", sig: "(string, string) -> bool", doc: "true if the path is in the JSON5 text", call: has },
        NativeFn { name: "getString", sig: "(string, string, string) -> string", doc: "the text at the path, or the fallback if the path is not there", call: get_string },
        NativeFn { name: "getInt", sig: "(string, string, int) -> int", doc: "the whole number at the path, or the fallback", call: get_int },
        NativeFn { name: "getFloat", sig: "(string, string, float) -> float", doc: "the number at the path (a whole number counts too), or the fallback", call: get_float },
        NativeFn { name: "getBool", sig: "(string, string, bool) -> bool", doc: "the true or false at the path, or the fallback", call: get_bool },
    ],
};

/// the next character that is neither a space nor part of a comment, from `i` on
fn next_significant(c: &[char], mut i: usize) -> Option<char> {
    loop {
        while i < c.len() && c[i].is_whitespace() {
            i += 1;
        }
        if c.get(i) == Some(&'/') && c.get(i + 1) == Some(&'/') {
            while i < c.len() && c[i] != '\n' {
                i += 1;
            }
        } else if c.get(i) == Some(&'/') && c.get(i + 1) == Some(&'*') {
            i += 2;
            while i < c.len() && !(c[i] == '*' && c.get(i + 1) == Some(&'/')) {
                i += 1;
            }
            i += 2;
        } else {
            return c.get(i).copied();
        }
    }
}

/// copies a single- or double-quoted JSON5 string as a JSON string. returns the index after it
fn copy_string(c: &[char], start: usize, out: &mut String) -> usize {
    let quote = c[start];
    out.push('"');
    let mut i = start + 1;
    while i < c.len() && c[i] != quote {
        let ch = c[i];
        if ch == '\\' && i + 1 < c.len() {
            let e = c[i + 1];
            match e {
                '\'' => out.push('\''),
                '"' => out.push_str("\\\""),
                '\n' => {}
                'x' if i + 3 < c.len() => {
                    let hex: String = c[i + 2..i + 4].iter().collect();
                    out.push_str(&format!("\\u00{}", hex));
                    i += 4;
                    continue;
                }
                _ => {
                    out.push('\\');
                    out.push(e);
                }
            }
            i += 2;
            continue;
        }
        if ch == '"' {
            out.push_str("\\\"");
        } else {
            out.push(ch);
        }
        i += 1;
    }
    out.push('"');
    i + 1
}

/// a JSON5 number as JSON: hex becomes decimal, a leading `+` goes, a leading `.` gets a `0`
fn number(token: &str) -> String {
    let (sign, body) = match token.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", token.strip_prefix('+').unwrap_or(token)),
    };
    if let Some(hex) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
        return match u64::from_str_radix(hex, 16) {
            Ok(n) => format!("{}{}", sign, n),
            Err(_) => token.to_string(), // too big or not hex: the JSON parser reports it
        };
    }
    if body.starts_with('.') {
        format!("{}0{}", sign, body)
    } else {
        format!("{}{}", sign, body)
    }
}

/// JSON5 as strict JSON: comments out, single quotes to double, names quoted, trailing commas
/// out, numbers plain. text inside strings is copied as it is, so a `//` in a URL stays
fn to_json(text: &str) -> String {
    let c: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        if ch == '/' && c.get(i + 1) == Some(&'/') {
            while i < c.len() && c[i] != '\n' {
                i += 1;
            }
        } else if ch == '/' && c.get(i + 1) == Some(&'*') {
            i += 2;
            while i < c.len() && !(c[i] == '*' && c.get(i + 1) == Some(&'/')) {
                i += 1;
            }
            i += 2;
            out.push(' ');
        } else if ch == '"' || ch == '\'' {
            i = copy_string(&c, i, &mut out);
        } else if ch.is_ascii_alphabetic() || ch == '_' || ch == '$' {
            let start = i;
            while i < c.len() && (c[i].is_ascii_alphanumeric() || c[i] == '_' || c[i] == '$') {
                i += 1;
            }
            let word: String = c[start..i].iter().collect();
            if next_significant(&c, i) == Some(':') {
                out.push('"');
                out.push_str(&word);
                out.push('"');
            } else {
                out.push_str(&word);
            }
        } else if ch.is_ascii_digit() || ((ch == '+' || ch == '-' || ch == '.') && c.get(i + 1).is_some_and(|n| n.is_ascii_digit() || *n == '.')) {
            let start = i;
            i += 1;
            while i < c.len()
                && (c[i].is_ascii_alphanumeric()
                    || c[i] == '.'
                    || ((c[i] == '+' || c[i] == '-') && (c[i - 1] == 'e' || c[i - 1] == 'E')))
            {
                i += 1;
            }
            let token: String = c[start..i].iter().collect();
            out.push_str(&number(&token));
        } else if ch == ',' {
            // a comma right before a closing bracket is a trailing comma: dropped
            if !matches!(next_significant(&c, i + 1), Some('}') | Some(']')) {
                out.push(',');
            }
            i += 1;
        } else {
            out.push(ch);
            i += 1;
        }
    }
    out
}

/// the first line of a message: the problem, not the drawing of where
fn first_line(text: &str) -> &str {
    text.lines().find(|l| !l.trim().is_empty()).unwrap_or(text).trim()
}

fn parse_text(function: &'static str, text: &str) -> Result<Json, LibError> {
    json::parse(&to_json(text)).map_err(|e| LibError::new(function, format!("not valid JSON5: {}", first_line(&e))))
}

fn text_arg(a: &[LibValue], i: usize) -> &str {
    match a.get(i) {
        Some(LibValue::Str(t)) => t.as_str(),
        _ => "",
    }
}

/// what a value is, in words for the message
fn describe(j: &Json) -> &'static str {
    match j {
        Json::Null => "null",
        Json::Bool(_) => "a boolean",
        Json::Number(_) => "a number",
        Json::Str(_) => "a string",
        Json::Array(_) => "an array",
        Json::Object(_) => "an object",
    }
}

/// the value at `path`, read by `pick`. `None` when the path is not in the text; a value of
/// another kind is an error that says what it is
fn read<T>(function: &'static str, text: &str, path: &str, wanted: &'static str, pick: fn(&Json) -> Option<T>) -> Result<Option<T>, LibError> {
    let root = parse_text(function, text)?;
    let segs = json::split_path(path).map_err(|e| LibError::new(function, e))?;
    let Ok(found) = json::lookup(&root, &segs) else {
        return Ok(None);
    };
    match pick(found) {
        Some(v) => Ok(Some(v)),
        None => Err(LibError::new(function, format!("the value at '{}' is {}, not {}", path, describe(found), wanted))),
    }
}

fn has(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    let root = parse_text("json5.has", text_arg(&a, 0))?;
    let segs = json::split_path(text_arg(&a, 1)).map_err(|e| LibError::new("json5.has", e))?;
    Ok(LibValue::Bool(json::lookup(&root, &segs).is_ok()))
}

fn get_string(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    let found = read("json5.getString", text_arg(&a, 0), text_arg(&a, 1), "a string", |j| match j {
        Json::Str(s) => Some(LibValue::Str(s.clone())),
        _ => None,
    })?;
    Ok(found.unwrap_or_else(|| LibValue::Str(text_arg(&a, 2).to_string())))
}

fn get_int(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    let found = read("json5.getInt", text_arg(&a, 0), text_arg(&a, 1), "an integer", |j| match j {
        Json::Number(n) => n.parse::<i64>().ok().map(LibValue::Int),
        _ => None,
    })?;
    Ok(found.unwrap_or_else(|| a.get(2).cloned().unwrap_or(LibValue::Int(0))))
}

fn get_float(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    let found = read("json5.getFloat", text_arg(&a, 0), text_arg(&a, 1), "a number", |j| match j {
        Json::Number(n) => n.parse::<f64>().ok().map(LibValue::Float),
        _ => None,
    })?;
    Ok(found.unwrap_or_else(|| a.get(2).cloned().unwrap_or(LibValue::Float(0.0))))
}

fn get_bool(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    let found = read("json5.getBool", text_arg(&a, 0), text_arg(&a, 1), "a boolean", |j| match j {
        Json::Bool(b) => Some(LibValue::Bool(*b)),
        _ => None,
    })?;
    Ok(found.unwrap_or_else(|| a.get(2).cloned().unwrap_or(LibValue::Bool(false))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LibFn;

    /// JSON5 on purpose: comments, unquoted and single-quoted names, hex, a leading `+` and `.`,
    /// a `//` inside a string, trailing commas
    const CFG: &str = "// configuracao\n{\n  nome: 'spg',  // sem aspas no nome\n  porta: 0x1F90,\n  mais: +5,\n  ponto: .5,\n  taxa: 0.5,\n  url: 'http://exemplo.com/a',\n  debug: true,\n  servidor: { host: \"local\", tls: { on: false }, },\n  portas: [8080, 8081,],\n}\n";

    fn call(f: LibFn, args: Vec<LibValue>) -> Result<LibValue, LibError> {
        f(&CallCtx { caller_file: None, caller_line: 0 }, args)
    }

    fn s(t: &str) -> LibValue {
        LibValue::Str(t.to_string())
    }

    #[test]
    fn reads_each_type_including_array_items_and_keeps_urls_whole() {
        assert_eq!(call(get_string, vec![s(CFG), s("nome"), s("")]).unwrap(), s("spg"));
        assert_eq!(call(get_string, vec![s(CFG), s("url"), s("")]).unwrap(), s("http://exemplo.com/a"));
        assert_eq!(call(get_int, vec![s(CFG), s("porta"), LibValue::Int(0)]).unwrap(), LibValue::Int(8080));
        assert_eq!(call(get_int, vec![s(CFG), s("mais"), LibValue::Int(0)]).unwrap(), LibValue::Int(5));
        assert_eq!(call(get_float, vec![s(CFG), s("ponto"), LibValue::Float(1.0)]).unwrap(), LibValue::Float(0.5));
        assert_eq!(call(get_bool, vec![s(CFG), s("servidor.tls.on"), LibValue::Bool(true)]).unwrap(), LibValue::Bool(false));
        assert_eq!(call(get_int, vec![s(CFG), s("portas.1"), LibValue::Int(0)]).unwrap(), LibValue::Int(8081));
    }

    #[test]
    fn a_missing_path_gives_the_fallback_and_has_says_so() {
        assert_eq!(call(get_string, vec![s(CFG), s("nao.existe"), s("padrao")]).unwrap(), s("padrao"));
        assert_eq!(call(get_int, vec![s(CFG), s("portas.9"), LibValue::Int(3)]).unwrap(), LibValue::Int(3));
        assert_eq!(call(has, vec![s(CFG), s("servidor.tls")]).unwrap(), LibValue::Bool(true));
        assert_eq!(call(has, vec![s(CFG), s("servidor.tls.off")]).unwrap(), LibValue::Bool(false));
    }

    #[test]
    fn a_wrong_type_or_bad_json5_is_an_error_with_the_reason() {
        let err = call(get_int, vec![s(CFG), s("nome"), LibValue::Int(0)]).unwrap_err();
        assert_eq!(err.function, "json5.getInt");
        assert!(err.reason.contains("a string, not an integer"), "{}", err.reason);
        let err = call(get_string, vec![s("{ a: "), s("a"), s("")]).unwrap_err();
        assert!(err.reason.starts_with("not valid JSON5:"), "{}", err.reason);
        assert!(!err.reason.contains('\n'));
    }
}
