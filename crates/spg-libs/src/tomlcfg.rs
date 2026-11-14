//! `toml`: read values from TOML text, the usual config format (`load #toml`). a value is
//! found by its dotted path (`"server.port"`). a key that is not there gives back the fallback
//! the program passed. text that is not TOML, or a value of another type, is an error the
//! program can catch. read only: programs read their config, they do not write it.

use crate::{CallCtx, Kind, LibError, LibValue, NativeFn, NativeModule};

pub static MODULE: NativeModule = NativeModule {
    kind: Kind::Load,
    name: "toml",
    doc: "read values from TOML text by their dotted path (load #toml)",
    structs: &[],
    objects: &[],
    functions: &[
        NativeFn { name: "has", sig: "(string, string) -> bool", doc: "true if the path is in the TOML text", call: has },
        NativeFn { name: "getString", sig: "(string, string, string) -> string", doc: "the text at the path, or the fallback if the path is not there", call: get_string },
        NativeFn { name: "getInt", sig: "(string, string, int) -> int", doc: "the whole number at the path, or the fallback", call: get_int },
        NativeFn { name: "getFloat", sig: "(string, string, float) -> float", doc: "the number at the path (a whole number counts too), or the fallback", call: get_float },
        NativeFn { name: "getBool", sig: "(string, string, bool) -> bool", doc: "the true or false at the path, or the fallback", call: get_bool },
    ],
};

/// the first line of a parser message: it names the problem, the rest is the caret drawing
fn first_line(text: &str) -> &str {
    text.lines().find(|l| !l.trim().is_empty()).unwrap_or(text).trim()
}

/// the value at `path`, or None when the path is not in the text. a path through an array or a
/// plain value is simply not there. text that is not TOML is an error
fn find(function: &'static str, text: &str, path: &str) -> Result<Option<toml::Value>, LibError> {
    let doc: toml::Table = text
        .parse()
        .map_err(|e: toml::de::Error| LibError::new(function, format!("not valid TOML: {}", first_line(&e.to_string()))))?;
    let mut keys = path.split('.');
    let first = match keys.next().and_then(|k| doc.get(k)) {
        Some(v) => v.clone(),
        None => return Ok(None),
    };
    let mut current = first;
    for key in keys {
        match current.as_table().and_then(|t| t.get(key)) {
            Some(v) => current = v.clone(),
            None => return Ok(None),
        }
    }
    Ok(Some(current))
}

fn text_arg(a: &[LibValue], i: usize) -> &str {
    match a.get(i) {
        Some(LibValue::Str(t)) => t.as_str(),
        _ => "",
    }
}

/// the value at the path, read as the kind the caller asked for. `None` if the path is missing
fn typed<'a>(function: &'static str, a: &'a [LibValue], wanted: &'static str, pick: fn(&toml::Value) -> Option<LibValue>) -> Result<Option<LibValue>, LibError> {
    let path = text_arg(a, 1);
    match find(function, text_arg(a, 0), path)? {
        None => Ok(None),
        Some(v) => match pick(&v) {
            Some(out) => Ok(Some(out)),
            None => Err(LibError::new(function, format!("the value at '{}' is {}, not {}", path, article(v.type_str()), wanted))),
        },
    }
}

/// "a string" / "an integer" for the message
fn article(type_name: &str) -> String {
    let first = type_name.chars().next().unwrap_or('x');
    let a = if "aeiou".contains(first) { "an" } else { "a" };
    format!("{} {}", a, type_name)
}

fn has(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    Ok(LibValue::Bool(find("toml.has", text_arg(&a, 0), text_arg(&a, 1))?.is_some()))
}

fn get_string(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    let found = typed("toml.getString", &a, "a string", |v| v.as_str().map(|s| LibValue::Str(s.to_string())))?;
    Ok(found.unwrap_or_else(|| LibValue::Str(text_arg(&a, 2).to_string())))
}

fn get_int(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    let found = typed("toml.getInt", &a, "an integer", |v| v.as_integer().map(LibValue::Int))?;
    Ok(found.unwrap_or_else(|| a.get(2).cloned().unwrap_or(LibValue::Int(0))))
}

fn get_float(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    let found = typed("toml.getFloat", &a, "a number", |v| {
        v.as_float().or_else(|| v.as_integer().map(|n| n as f64)).map(LibValue::Float)
    })?;
    Ok(found.unwrap_or_else(|| a.get(2).cloned().unwrap_or(LibValue::Float(0.0))))
}

fn get_bool(_: &CallCtx, a: Vec<LibValue>) -> Result<LibValue, LibError> {
    let found = typed("toml.getBool", &a, "a boolean", |v| v.as_bool().map(LibValue::Bool))?;
    Ok(found.unwrap_or_else(|| a.get(2).cloned().unwrap_or(LibValue::Bool(false))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LibFn;

    const CFG: &str = "name = \"spg\"\nport = 8080\nratio = 0.5\ndebug = true\n\n[server]\nhost = \"local\"\n\n[server.tls]\non = false\n";

    fn call(f: LibFn, args: Vec<LibValue>) -> Result<LibValue, LibError> {
        f(&CallCtx { caller_file: None, caller_line: 0 }, args)
    }

    fn s(t: &str) -> LibValue {
        LibValue::Str(t.to_string())
    }

    #[test]
    fn reads_each_type_by_its_dotted_path() {
        assert_eq!(call(get_string, vec![s(CFG), s("server.host"), s("x")]).unwrap(), s("local"));
        assert_eq!(call(get_int, vec![s(CFG), s("port"), LibValue::Int(1)]).unwrap(), LibValue::Int(8080));
        assert_eq!(call(get_float, vec![s(CFG), s("ratio"), LibValue::Float(1.0)]).unwrap(), LibValue::Float(0.5));
        // a whole number reads as a float too
        assert_eq!(call(get_float, vec![s(CFG), s("port"), LibValue::Float(1.0)]).unwrap(), LibValue::Float(8080.0));
        assert_eq!(call(get_bool, vec![s(CFG), s("server.tls.on"), LibValue::Bool(true)]).unwrap(), LibValue::Bool(false));
    }

    #[test]
    fn a_missing_path_gives_the_fallback_and_has_says_so() {
        assert_eq!(call(get_string, vec![s(CFG), s("nope.here"), s("padrão")]).unwrap(), s("padrão"));
        assert_eq!(call(get_int, vec![s(CFG), s("server.port"), LibValue::Int(3000)]).unwrap(), LibValue::Int(3000));
        assert_eq!(call(has, vec![s(CFG), s("server.tls")]).unwrap(), LibValue::Bool(true));
        assert_eq!(call(has, vec![s(CFG), s("server.tls.off")]).unwrap(), LibValue::Bool(false));
    }

    #[test]
    fn a_wrong_type_or_bad_toml_is_an_error_with_the_reason() {
        let err = call(get_int, vec![s(CFG), s("name"), LibValue::Int(0)]).unwrap_err();
        assert_eq!(err.function, "toml.getInt");
        assert!(err.reason.contains("a string, not an integer"), "{}", err.reason);
        let err = call(get_string, vec![s("x = = 1"), s("x"), s("")]).unwrap_err();
        assert!(err.reason.starts_with("not valid TOML:"), "{}", err.reason);
        assert!(!err.reason.contains('\n'));
    }
}
