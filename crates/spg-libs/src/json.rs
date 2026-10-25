//! `@json.*`: JSON text <-> values. written by hand, no crate. we did it.
//! part of the core (always there, written with `@`).
//!
//! spg has no dynamic "json" type, so READING works on the text: a
//! query by path (`get`, `has`, `type`, `len`, `keys`) parses the text, walks
//! the path, answers with a string, bool or int. WRITING (`stringify`,
//! `pretty`, `set`) takes any value and serializes it by what it is. simple.
//!
//! paths are segments joined by `.`. a number indexes an array (on an
//! object it's just a key name). a literal dot or backslash in a key is
//! written `\.` / `\\`. empty path = the whole document. 

use crate::{CallCtx, Kind, LibError, LibValue, NativeFn, NativeModule};

pub static MODULE: NativeModule = NativeModule {
    kind: Kind::Core,
    name: "json",
    doc: "read and write JSON",
    structs: &[],
    objects: &[],
    functions: &[
        NativeFn { name: "stringify", sig: "(any) -> string", doc: "the value as compact JSON text", call: stringify_call },
        NativeFn { name: "pretty", sig: "(any) -> string", doc: "the value as indented JSON text", call: pretty_call },
        NativeFn { name: "get", sig: "(string, string) -> string", doc: "the value at a path inside a JSON text", call: get_call },
        NativeFn { name: "has", sig: "(string, string) -> bool", doc: "true if the path exists in a JSON text", call: has_call },
        NativeFn { name: "type", sig: "(string, string) -> string", doc: "the JSON type at a path: object, array, string, number, bool or null", call: type_call },
        NativeFn { name: "len", sig: "(string, string) -> int", doc: "number of items of the array or object at a path", call: len_call },
        NativeFn { name: "keys", sig: "(string, string) -> array<string>", doc: "the keys of the object at a path", call: keys_call },
        NativeFn { name: "valid", sig: "(string) -> bool", doc: "true if the text is valid JSON", call: valid_call },
        NativeFn { name: "set", sig: "(string, string, any) -> string", doc: "a new JSON text with the value written at a path", call: set_call },
    ],
};

type Out = Result<LibValue, LibError>;

fn text(args: &[LibValue], i: usize) -> &str {
    match args.get(i) {
        Some(LibValue::Str(t)) => t.as_str(),
        _ => "",
    }
}

fn stringify_call(_: &CallCtx, a: Vec<LibValue>) -> Out {
    stringify(&a[0], false).map(LibValue::Str).map_err(|m| LibError::new("@json.stringify", m))
}

fn pretty_call(_: &CallCtx, a: Vec<LibValue>) -> Out {
    stringify(&a[0], true).map(LibValue::Str).map_err(|m| LibError::new("@json.pretty", m))
}

fn get_call(_: &CallCtx, a: Vec<LibValue>) -> Out {
    get(text(&a, 0), text(&a, 1)).map(LibValue::Str).map_err(|m| LibError::new("@json.get", m))
}

fn has_call(_: &CallCtx, a: Vec<LibValue>) -> Out {
    has(text(&a, 0), text(&a, 1)).map(LibValue::Bool).map_err(|m| LibError::new("@json.has", m))
}

fn type_call(_: &CallCtx, a: Vec<LibValue>) -> Out {
    type_of(text(&a, 0), text(&a, 1)).map(|t| LibValue::Str(t.to_string())).map_err(|m| LibError::new("@json.type", m))
}

fn len_call(_: &CallCtx, a: Vec<LibValue>) -> Out {
    len(text(&a, 0), text(&a, 1)).map(|n| LibValue::Int(n as i64)).map_err(|m| LibError::new("@json.len", m))
}

fn keys_call(_: &CallCtx, a: Vec<LibValue>) -> Out {
    keys(text(&a, 0), text(&a, 1))
        .map(|ks| LibValue::Array(ks.into_iter().map(LibValue::Str).collect()))
        .map_err(|m| LibError::new("@json.keys", m))
}

fn valid_call(_: &CallCtx, a: Vec<LibValue>) -> Out {
    Ok(LibValue::Bool(valid(text(&a, 0))))
}

fn set_call(_: &CallCtx, a: Vec<LibValue>) -> Out {
    set(text(&a, 0), text(&a, 1), &a[2]).map(LibValue::Str).map_err(|m| LibError::new("@json.set", m))
}

/// deepest nesting accepted when parsing or serializing (the
/// latter also stops a struct that contains itself. no infinite loops, please.
const MAX_DEPTH: usize = 512;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Json {
    Null,
    Bool(bool),
    /// the number exactly as written (`1.50`, `1e5`), so editing a file doesn't
    /// rewrite its numbers. respect the user's `1.50`.
    Number(String),
    Str(String),
    Array(Vec<Json>),
    /// keys keep the order they were written in. order matters, apparently.
    Object(Vec<(String, Json)>),
}

fn type_name(j: &Json) -> &'static str {
    match j {
        Json::Null => "null",
        Json::Bool(_) => "bool",
        Json::Number(_) => "number",
        Json::Str(_) => "string",
        Json::Array(_) => "array",
        Json::Object(_) => "object",
    }
}

// ---------------------------------------------------------------- parsing (the fun part) --

struct Parser<'a> {
    src: &'a str,
    pos: usize,
}

pub(crate) fn parse(text: &str) -> Result<Json, String> {
    let mut p = Parser { src: text, pos: 0 };
    p.skip_ws();
    let value = p.value(0)?;
    p.skip_ws();
    if p.pos < p.src.len() {
        return Err(p.error("unexpected data after the end of the JSON value"));
    }
    Ok(value)
}

impl<'a> Parser<'a> {
    fn error(&self, msg: &str) -> String {
        let before = self.src.get(..self.pos).unwrap_or(self.src);
        let line = before.matches('\n').count() + 1;
        let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
        format!("invalid JSON: {} (line {}, column {})", msg, line, col)
    }

    fn peek(&self) -> Option<char> {
        self.src[self.pos..].chars().next()
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t' | '\n' | '\r')) {
            self.pos += 1;
        }
    }

    fn value(&mut self, depth: usize) -> Result<Json, String> {
        if depth > MAX_DEPTH {
            return Err(self.error("nested too deeply"));
        }
        match self.peek() {
            None => Err(self.error("unexpected end of the text")),
            Some('{') => self.object(depth),
            Some('[') => self.array(depth),
            Some('"') => Ok(Json::Str(self.string()?)),
            Some('t') => self.literal("true", Json::Bool(true)),
            Some('f') => self.literal("false", Json::Bool(false)),
            Some('n') => self.literal("null", Json::Null),
            Some('-' | '0'..='9') => self.number(),
            Some(c) => Err(self.error(&format!("unexpected character '{}'", c))),
        }
    }

    fn literal(&mut self, word: &str, value: Json) -> Result<Json, String> {
        if self.src[self.pos..].starts_with(word) {
            self.pos += word.len();
            Ok(value)
        } else {
            Err(self.error(&format!("expected '{}'", word)))
        }
    }

    fn digits(&mut self) -> usize {
        let start = self.pos;
        while matches!(self.peek(), Some('0'..='9')) {
            self.pos += 1;
        }
        self.pos - start
    }

    fn number(&mut self) -> Result<Json, String> {
        let start = self.pos;
        if self.peek() == Some('-') {
            self.pos += 1;
        }
        match self.peek() {
            Some('0') => self.pos += 1,
            Some('1'..='9') => {
                self.digits();
            }
            _ => return Err(self.error("invalid number")),
        }
        if self.peek() == Some('.') {
            self.pos += 1;
            if self.digits() == 0 {
                return Err(self.error("invalid number: digits expected after '.'"));
            }
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some('+' | '-')) {
                self.pos += 1;
            }
            if self.digits() == 0 {
                return Err(self.error("invalid number: digits expected in the exponent"));
            }
        }
        Ok(Json::Number(self.src[start..self.pos].to_string()))
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let h = self.src.get(self.pos..self.pos + 4).filter(|h| h.chars().all(|c| c.is_ascii_hexdigit()));
        match h {
            Some(h) => {
                self.pos += 4;
                Ok(u32::from_str_radix(h, 16).expect("four hex digits"))
            }
            None => Err(self.error("invalid \\u escape (four hex digits expected)")),
        }
    }

    fn string(&mut self) -> Result<String, String> {
        self.pos += 1; // opening quote
        let mut out = String::new();
        loop {
            let Some(c) = self.peek() else { return Err(self.error("unterminated string")) };
            match c {
                '"' => {
                    self.pos += 1;
                    return Ok(out);
                }
                '\\' => {
                    self.pos += 1;
                    let Some(e) = self.peek() else { return Err(self.error("unterminated string")) };
                    match e {
                        '"' | '\\' | '/' => out.push(e),
                        'b' => out.push('\u{8}'),
                        'f' => out.push('\u{c}'),
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        'u' => {
                            self.pos += 1;
                            let first = self.hex4()?;
                            let cp = if (0xD800..0xDC00).contains(&first) {
                                if !self.src[self.pos..].starts_with("\\u") {
                                    return Err(self.error("a high surrogate must be followed by a low one"));
                                }
                                self.pos += 2;
                                let low = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&low) {
                                    return Err(self.error("invalid surrogate pair"));
                                }
                                0x10000 + ((first - 0xD800) << 10) + (low - 0xDC00)
                            } else if (0xDC00..0xE000).contains(&first) {
                                return Err(self.error("a low surrogate without its high one"));
                            } else {
                                first
                            };
                            out.push(char::from_u32(cp).ok_or_else(|| self.error("invalid code point"))?);
                            continue; // the escape is already consumed
                        }
                        _ => return Err(self.error("invalid escape in a string")),
                    }
                    self.pos += e.len_utf8();
                }
                c if (c as u32) < 0x20 => {
                    return Err(self.error("control character inside a string (write it as \\n, \\t, ...)"))
                }
                c => {
                    out.push(c);
                    self.pos += c.len_utf8();
                }
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Json, String> {
        self.pos += 1; // [
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(']') {
            self.pos += 1;
            return Ok(Json::Array(items));
        }
        loop {
            self.skip_ws();
            items.push(self.value(depth + 1)?);
            self.skip_ws();
            match self.peek() {
                Some(',') => self.pos += 1,
                Some(']') => {
                    self.pos += 1;
                    return Ok(Json::Array(items));
                }
                _ => return Err(self.error("expected ',' or ']'")),
            }
        }
    }

    fn object(&mut self, depth: usize) -> Result<Json, String> {
        self.pos += 1; // {
        let mut entries: Vec<(String, Json)> = Vec::new();
        self.skip_ws();
        if self.peek() == Some('}') {
            self.pos += 1;
            return Ok(Json::Object(entries));
        }
        loop {
            self.skip_ws();
            if self.peek() != Some('"') {
                return Err(self.error("expected a string key"));
            }
            let key = self.string()?;
            self.skip_ws();
            if self.peek() != Some(':') {
                return Err(self.error("expected ':' after the key"));
            }
            self.pos += 1;
            self.skip_ws();
            let value = self.value(depth + 1)?;
            // a repeated key keeps its first position and takes the last value. classic.
            match entries.iter().position(|(k, _)| *k == key) {
                Some(i) => entries[i].1 = value,
                None => entries.push((key, value)),
            }
            self.skip_ws();
            match self.peek() {
                Some(',') => self.pos += 1,
                Some('}') => {
                    self.pos += 1;
                    return Ok(Json::Object(entries));
                }
                _ => return Err(self.error("expected ',' or '}'")),
            }
        }
    }
}

// ----------------------------------------------------------- serializing (lots of quotes) --

fn write_str(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn newline(out: &mut String, indent: Option<&str>, level: usize) {
    if let Some(unit) = indent {
        out.push('\n');
        for _ in 0..level {
            out.push_str(unit);
        }
    }
}

/// `indent` None = compact. Some(unit) = one item per line, `unit` per level.
fn write_json(j: &Json, out: &mut String, indent: Option<&str>, level: usize) {
    match j {
        Json::Null => out.push_str("null"),
        Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Json::Number(n) => out.push_str(n),
        Json::Str(s) => write_str(s, out),
        Json::Array(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                newline(out, indent, level + 1);
                write_json(item, out, indent, level + 1);
            }
            newline(out, indent, level);
            out.push(']');
        }
        Json::Object(entries) => {
            if entries.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push('{');
            for (i, (k, v)) in entries.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                newline(out, indent, level + 1);
                write_str(k, out);
                out.push(':');
                if indent.is_some() {
                    out.push(' ');
                }
                write_json(v, out, indent, level + 1);
            }
            newline(out, indent, level);
            out.push('}');
        }
    }
}

fn compact(j: &Json) -> String {
    let mut out = String::new();
    write_json(j, &mut out, None, 0);
    out
}

fn from_value(v: &LibValue, depth: usize) -> Result<Json, String> {
    if depth > MAX_DEPTH {
        return Err("the value is nested too deeply (does it contain itself?)".to_string());
    }
    Ok(match v {
        LibValue::Handler(_) => return Err("a function has no JSON form".to_string()),
        LibValue::Variant(enum_name, variant) => return Err(format!("{enum_name}.{variant} has no JSON form")),
        LibValue::Opaque("function") => return Err("a function has no JSON form".to_string()),
        LibValue::Opaque("task") => return Err("a task has no JSON form (call .wait() first)".to_string()),
        LibValue::Opaque(_) => return Err("the value is nested too deeply (does it contain itself?)".to_string()),
        LibValue::Nil => Json::Null,
        LibValue::Int(n) => Json::Number(n.to_string()),
        LibValue::Float(x) => {
            if !x.is_finite() {
                return Err(format!("{} has no JSON form (JSON has no NaN or infinity)", x));
            }
            // `{:?}` keeps `1.0` a float (`{}` would write `1`, which is a lie)
            Json::Number(format!("{:?}", x))
        }
        LibValue::Bool(b) => Json::Bool(*b),
        LibValue::Str(s) => Json::Str(s.to_string()),
        LibValue::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(from_value(item, depth + 1)?);
            }
            Json::Array(out)
        }
        LibValue::Map(pairs) => {
            let mut entries = Vec::new();
            for (k, val) in pairs {
                // JSON keys are strings. an int/bool/enum key gets written as its text.
                let key = match k {
                    LibValue::Str(s) => s.to_string(),
                    LibValue::Int(n) => n.to_string(),
                    LibValue::Bool(b) => b.to_string(),
                    LibValue::Float(x) => x.to_string(),
                    LibValue::Enum(text) => text.to_string(),
                    _ => return Err("a map key has no JSON form".to_string()),
                };
                entries.push((key, from_value(val, depth + 1)?));
            }
            Json::Object(entries)
        }
        LibValue::Struct { fields, .. } => {
            let mut entries = Vec::new();
            for (name, val) in fields {
                entries.push((name.to_string(), from_value(val, depth + 1)?));
            }
            Json::Object(entries)
        }
        LibValue::Enum(text) => Json::Str(text.to_string()),
        LibValue::Void => return Err("void has no JSON form".to_string()),
    })
}

pub(crate) fn stringify(v: &LibValue, pretty: bool) -> Result<String, String> {
    let j = from_value(v, 0)?;
    let mut out = String::new();
    write_json(&j, &mut out, if pretty { Some("  ") } else { None }, 0);
    Ok(out)
}

// ------------------------------------------------------------------ paths (ugh) --

pub(crate) fn split_path(path: &str) -> Result<Vec<String>, String> {
    if path.is_empty() {
        return Ok(Vec::new());
    }
    let mut segs = vec![String::new()];
    let mut chars = path.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some(n @ ('.' | '\\')) => segs.last_mut().expect("one segment").push(n),
                Some(n) => {
                    let last = segs.last_mut().expect("one segment");
                    last.push('\\');
                    last.push(n);
                }
                None => segs.last_mut().expect("one segment").push('\\'),
            },
            '.' => segs.push(String::new()),
            c => segs.last_mut().expect("one segment").push(c),
        }
    }
    if segs.iter().any(|s| s.is_empty()) {
        return Err(format!("the path '{}' has an empty step", path));
    }
    Ok(segs)
}

fn describe(walked: &[String], j: &Json) -> String {
    if walked.is_empty() {
        format!("the root {}", type_name(j))
    } else {
        format!("the {} at '{}'", type_name(j), walked.join("."))
    }
}

pub(crate) fn lookup<'a>(root: &'a Json, segs: &[String]) -> Result<&'a Json, String> {
    let mut cur = root;
    for (i, seg) in segs.iter().enumerate() {
        let walked = &segs[..i];
        let through = segs[..=i].join(".");
        cur = match cur {
            Json::Object(entries) => match entries.iter().find(|(k, _)| k == seg) {
                Some((_, v)) => v,
                None => {
                    return Err(format!(
                        "path not found: '{}' ({} has no key '{}')",
                        through,
                        describe(walked, cur),
                        seg
                    ))
                }
            },
            Json::Array(items) => {
                let idx = seg.parse::<usize>().map_err(|_| {
                    format!("path not found: '{}' ({} needs a numeric index, not '{}')", through, describe(walked, cur), seg)
                })?;
                match items.get(idx) {
                    Some(v) => v,
                    None => {
                        return Err(format!(
                            "path not found: '{}' (index {} is out of range: {} has {} item(s))",
                            through,
                            idx,
                            describe(walked, cur),
                            items.len()
                        ))
                    }
                }
            }
            other => {
                return Err(format!(
                    "path not found: '{}' ({} is not an object or an array, so it has no '{}')",
                    through,
                    describe(walked, other),
                    seg
                ))
            }
        };
    }
    Ok(cur)
}

// ------------------------------------------------------------- the builtins (finally) --

pub(crate) fn valid(text: &str) -> bool {
    parse(text).is_ok()
}

fn at(text: &str, path: &str) -> Result<Json, String> {
    let root = parse(text)?;
    let segs = split_path(path)?;
    lookup(&root, &segs).cloned()
}

/// strings come back decoded, numbers and bools as text, `null` as
/// `null`. an object or array comes back as compact JSON (so it can be
/// queried again. turtles all the way down.)
pub(crate) fn get(text: &str, path: &str) -> Result<String, String> {
    Ok(match at(text, path)? {
        Json::Str(s) => s,
        Json::Number(n) => n,
        Json::Bool(b) => b.to_string(),
        Json::Null => "null".to_string(),
        other => compact(&other),
    })
}

/// a path that leads nowhere is `false`. an invalid text or malformed path
/// is still an error. no free passes.
pub(crate) fn has(text: &str, path: &str) -> Result<bool, String> {
    let root = parse(text)?;
    let segs = split_path(path)?;
    Ok(lookup(&root, &segs).is_ok())
}

pub(crate) fn type_of(text: &str, path: &str) -> Result<&'static str, String> {
    Ok(type_name(&at(text, path)?))
}

pub(crate) fn len(text: &str, path: &str) -> Result<usize, String> {
    match at(text, path)? {
        Json::Array(items) => Ok(items.len()),
        Json::Object(entries) => Ok(entries.len()),
        other => Err(format!("'{}' is a {}: len only measures arrays and objects", path, type_name(&other))),
    }
}

pub(crate) fn keys(text: &str, path: &str) -> Result<Vec<String>, String> {
    match at(text, path)? {
        Json::Object(entries) => Ok(entries.into_iter().map(|(k, _)| k).collect()),
        other => Err(format!("'{}' is a {}: keys only lists the keys of an object", path, type_name(&other))),
    }
}

fn assign(node: &mut Json, segs: &[String], new: Json, walked: &[String]) -> Result<(), String> {
    let (seg, rest) = segs.split_first().expect("a non-empty path");
    let through = {
        let mut t = walked.to_vec();
        t.push(seg.clone());
        t.join(".")
    };
    let here = |j: &Json| describe(walked, j);
    match node {
        Json::Object(entries) => {
            let pos = entries.iter().position(|(k, _)| k == seg);
            if rest.is_empty() {
                match pos {
                    Some(i) => entries[i].1 = new,
                    None => entries.push((seg.clone(), new)),
                }
                Ok(())
            } else {
                match pos {
                    Some(i) => {
                        let mut next = walked.to_vec();
                        next.push(seg.clone());
                        assign(&mut entries[i].1, rest, new, &next)
                    }
                    None => Err(format!(
                        "path not found: '{}' (set only creates the LAST step of a path; the object has no key '{}')",
                        through, seg
                    )),
                }
            }
        }
        Json::Array(items) => {
            let desc = here(&Json::Array(Vec::new()));
            let idx = seg
                .parse::<usize>()
                .map_err(|_| format!("path not found: '{}' ({} needs a numeric index, not '{}')", through, desc, seg))?;
            if rest.is_empty() {
                if idx < items.len() {
                    items[idx] = new;
                } else if idx == items.len() {
                    items.push(new); // one past the end appends
                } else {
                    return Err(format!(
                        "path not found: '{}' (index {} is out of range: {} has {} item(s))",
                        through,
                        idx,
                        desc,
                        items.len()
                    ));
                }
                Ok(())
            } else {
                let len = items.len();
                match items.get_mut(idx) {
                    Some(n) => {
                        let mut next = walked.to_vec();
                        next.push(seg.clone());
                        assign(n, rest, new, &next)
                    }
                    None => Err(format!(
                        "path not found: '{}' (index {} is out of range: {} has {} item(s))",
                        through, idx, desc, len
                    )),
                }
            }
        }
        other => Err(format!(
            "path not found: '{}' ({} is not an object or an array, so it has no '{}')",
            through,
            here(other),
            seg
        )),
    }
}

/// the indentation the file already uses (first indented line). 2 spaces if none.
fn detect_indent(text: &str) -> String {
    for line in text.lines().skip(1) {
        let ws: String = line.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
        if !ws.is_empty() && ws.len() < line.len() {
            return ws;
        }
    }
    "  ".to_string()
}

/// the doc is written back in the style it came in: compact if it was on
/// one line, otherwise one item per line, same indentation. a final
/// newline and `\r\n` line endings are kept. we respect your files.
fn render_like(original: &str, j: &Json) -> String {
    let multiline = original.trim_end().contains('\n');
    let mut out = String::new();
    if multiline {
        let unit = detect_indent(original);
        write_json(j, &mut out, Some(&unit), 0);
    } else {
        write_json(j, &mut out, None, 0);
    }
    if original.ends_with('\n') {
        out.push('\n');
    }
    if original.contains("\r\n") {
        out = out.replace('\n', "\r\n");
    }
    out
}

/// returns the whole doc with `value` written at `path`. only the LAST
/// step of the path may be new (a key gets added. one past the end of an array
/// appends). the doc gets rewritten, so spacing and `\uXXXX`
/// escapes get normalized. (blame the rewrite, not me.)
pub(crate) fn set(text: &str, path: &str, value: &LibValue) -> Result<String, String> {
    let mut root = parse(text)?;
    let new = from_value(value, 0)?;
    let segs = split_path(path)?;
    if segs.is_empty() {
        root = new;
    } else {
        assign(&mut root, &segs, new, &[])?;
    }
    Ok(render_like(text, &root))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = r#"{"name":"spg","n":3,"pi":1.50,"ok":true,"none":null,
        "tags":["a","b"],"nested":{"x":1,"a.b":"dot","0":"zero"},"é":"\u00e9\n"}"#;

    fn s(v: &str) -> LibValue {
        LibValue::Str(v.to_string())
    }

    #[test]
    fn get_decodes_strings_and_keeps_numbers_as_written() {
        assert_eq!(get(DOC, "name").unwrap(), "spg");
        assert_eq!(get(DOC, "pi").unwrap(), "1.50");
        assert_eq!(get(DOC, "ok").unwrap(), "true");
        assert_eq!(get(DOC, "none").unwrap(), "null");
        assert_eq!(get(DOC, "é").unwrap(), "é\n");
        assert_eq!(get(DOC, "tags").unwrap(), r#"["a","b"]"#);
        assert_eq!(get(DOC, "tags.1").unwrap(), "b");
        assert_eq!(get(r#"[[1,2],[3]]"#, "0.1").unwrap(), "2");
        assert_eq!(get("42", "").unwrap(), "42");
    }

    #[test]
    fn paths_escape_dots_and_treat_digits_on_objects_as_keys() {
        assert_eq!(get(DOC, r"nested.a\.b").unwrap(), "dot");
        assert_eq!(get(DOC, "nested.0").unwrap(), "zero");
        assert!(split_path("a..b").is_err());
        assert!(split_path("a.").is_err());
    }

    #[test]
    fn has_type_len_keys() {
        assert!(has(DOC, "nested.x").unwrap());
        assert!(!has(DOC, "nested.q").unwrap());
        assert!(!has(DOC, "name.deeper").unwrap());
        assert!(!has(DOC, "tags.9").unwrap());
        assert_eq!(type_of(DOC, "tags").unwrap(), "array");
        assert_eq!(type_of(DOC, "none").unwrap(), "null");
        assert_eq!(type_of(DOC, "pi").unwrap(), "number");
        assert_eq!(len(DOC, "tags").unwrap(), 2);
        assert_eq!(len(DOC, "").unwrap(), 8);
        assert!(len(DOC, "name").is_err());
        assert_eq!(keys(DOC, "nested").unwrap(), vec!["x", "a.b", "0"]);
        assert!(keys(DOC, "tags").is_err());
    }

    #[test]
    fn missing_paths_say_where_they_stopped() {
        let e = get(DOC, "nested.q").unwrap_err();
        assert!(e.contains("path not found: 'nested.q'") && e.contains("no key 'q'"), "{}", e);
        let e = get(DOC, "tags.5").unwrap_err();
        assert!(e.contains("out of range") && e.contains("2 item(s)"), "{}", e);
        let e = get(DOC, "name.x").unwrap_err();
        assert!(e.contains("is not an object or an array"), "{}", e);
    }

    #[test]
    fn invalid_json_is_reported_with_its_position() {
        for (bad, want) in [
            ("{\"a\":1,}", "expected a string key"),
            ("[1,]", "unexpected character ']'"),
            ("{\"a\" 1}", "expected ':'"),
            ("[1 2]", "expected ',' or ']'"),
            ("\"abc", "unterminated string"),
            ("\"a\\qb\"", "invalid escape"),
            ("01", "unexpected data"),
            ("1.", "digits expected after '.'"),
            ("tru", "expected 'true'"),
            ("{} x", "unexpected data after"),
            ("\"a\nb\"", "control character"),
            ("", "unexpected end"),
        ] {
            let e = parse(bad).unwrap_err();
            assert!(e.contains(want), "{:?}: {}", bad, e);
        }
        let e = parse("{\n  \"a\": ?\n}").unwrap_err();
        assert!(e.contains("line 2, column 8"), "{}", e);
        assert!(valid("[]") && valid(" {\"a\": [1, 2.5e-3, -0]} ") && !valid("{oops"));
    }

    #[test]
    fn unicode_escapes_and_surrogate_pairs() {
        assert_eq!(get(r#"{"a":"\ud83d\ude00 \u00e9"}"#, "a").unwrap(), "😀 é");
        assert!(parse(r#""\ud83d""#).is_err());
        assert!(parse(r#""\ude00""#).is_err());
    }

    #[test]
    fn nesting_is_limited_instead_of_overflowing_the_stack() {
        let deep = format!("{}{}", "[".repeat(MAX_DEPTH + 5), "]".repeat(MAX_DEPTH + 5));
        assert!(parse(&deep).unwrap_err().contains("nested too deeply"));
        let ok = format!("{}{}", "[".repeat(100), "]".repeat(100));
        assert!(valid(&ok));
    }

    #[test]
    fn stringify_scalars_and_arrays() {
        assert_eq!(stringify(&LibValue::Int(-3), false).unwrap(), "-3");
        assert_eq!(stringify(&LibValue::Float(1.0), false).unwrap(), "1.0");
        assert_eq!(stringify(&LibValue::Float(9.5), false).unwrap(), "9.5");
        assert_eq!(stringify(&LibValue::Bool(true), false).unwrap(), "true");
        assert_eq!(stringify(&s("a\"b\n\u{1}"), false).unwrap(), "\"a\\\"b\\n\\u0001\"");
        let arr = LibValue::Array(vec![LibValue::Int(1), s("x")]);
        assert_eq!(stringify(&arr, false).unwrap(), r#"[1,"x"]"#);
        assert_eq!(stringify(&arr, true).unwrap(), "[\n  1,\n  \"x\"\n]");
        assert!(stringify(&LibValue::Float(f64::NAN), false).unwrap_err().contains("no JSON form"));
        assert!(stringify(&LibValue::Void, false).is_err());
        let empty = LibValue::Array(vec![]);
        assert_eq!(stringify(&empty, true).unwrap(), "[]");
    }

    #[test]
    fn set_replaces_adds_and_appends_in_the_documents_own_style() {
        // compact stays compact. good boy.
        assert_eq!(set(r#"{"a":1}"#, "a", &LibValue::Int(2)).unwrap(), r#"{"a":2}"#);
        assert_eq!(set(r#"{"a":1}"#, "b", &s("x")).unwrap(), r#"{"a":1,"b":"x"}"#);
        assert_eq!(set(r#"{"a":[1]}"#, "a.1", &LibValue::Int(2)).unwrap(), r#"{"a":[1,2]}"#);
        assert_eq!(set(r#"{"a":[1,2]}"#, "a.0", &LibValue::Int(9)).unwrap(), r#"{"a":[9,2]}"#);
        assert_eq!(set("1", "", &LibValue::Int(2)).unwrap(), "2");
        // 4 spaces, final newline and CRLF are kept. the file is safe.
        assert_eq!(set("{\n    \"a\": 1\n}\n", "a", &LibValue::Int(2)).unwrap(), "{\n    \"a\": 2\n}\n");
        assert_eq!(set("{\n\t\"a\": 1\n}", "a", &LibValue::Int(2)).unwrap(), "{\n\t\"a\": 2\n}");
        assert_eq!(set("{\r\n  \"a\": 1\r\n}", "a", &LibValue::Int(2)).unwrap(), "{\r\n  \"a\": 2\r\n}");
        // a one-line file with a final newline is not "multi-line". rules are rules.
        assert_eq!(set("{\"a\":1}\n", "a", &LibValue::Int(2)).unwrap(), "{\"a\":2}\n");
        // numbers elsewhere are untouched. not my business.
        assert_eq!(set(r#"{"a":1.50,"b":0}"#, "b", &LibValue::Int(1)).unwrap(), r#"{"a":1.50,"b":1}"#);
    }

    #[test]
    fn set_only_creates_the_last_step() {
        let e = set("{}", "a.b", &LibValue::Int(1)).unwrap_err();
        assert!(e.contains("only creates the LAST step"), "{}", e);
        assert!(set(r#"{"a":[1]}"#, "a.5", &LibValue::Int(1)).unwrap_err().contains("out of range"));
        assert!(set(r#"{"a":1}"#, "a.b", &LibValue::Int(1)).unwrap_err().contains("not an object or an array"));
        assert!(set("{oops", "a", &LibValue::Int(1)).unwrap_err().contains("invalid JSON"));
    }
}
