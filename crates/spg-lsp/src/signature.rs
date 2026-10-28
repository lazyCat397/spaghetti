//! function signatures for the editor, read from the document's tokens.
//!
//! three kinds of callable names are understood:
//! - named functions: `function dobro(x: int) -> int`
//! - values of a function type: `let f: function(int) -> int`, and a param
//!   `g: function(int) -> int` (struct fields have the same shape, harmless here)
//! - lambdas bound to a name: `let s = function(x: int) -> int { ... }`
//!
//! everything is token-level on purpose, like the rest of the editor support. it
//! works while the buffer is half-typed, and never needs a full resolve. sturdy.

use spg_lexer::{Token, TokenKind as T};

/// a callable: its name, its params as (name, type text), and its return type
#[derive(Debug, Clone, PartialEq)]
pub struct Signature {
    pub name: String,
    pub params: Vec<(String, String)>,
    pub ret: String,
}

impl Signature {
    /// `name(a: int, b: string) -> bool`. the param names are left out for
    /// function-type values, which only have types (`f(int) -> int`)
    pub fn label(&self) -> String {
        let ps: Vec<String> = self
            .params
            .iter()
            .map(|(n, t)| if n.is_empty() { t.clone() } else { format!("{}: {}", n, t) })
            .collect();
        format!("{}({}) -> {}", self.name, ps.join(", "), self.ret)
    }
}

fn kind(tokens: &[Token], i: usize) -> Option<&T> {
    tokens.get(i).map(|t| &t.kind)
}

/// index of the `)` that closes the `(` at `open`
fn closing(tokens: &[Token], open: usize) -> Option<usize> {
    let mut nest = 0usize;
    for (i, t) in tokens.iter().enumerate().skip(open) {
        match t.kind {
            T::LParen => nest += 1,
            T::RParen => {
                nest = nest.checked_sub(1)?;
                if nest == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// reads one type starting at `i`, returning its text and the index after it.
/// covers the names, `array<T>`, `map<K, V>` and `function(A, B) -> R`
pub fn read_type(tokens: &[Token], i: usize) -> (String, usize) {
    let (base, next) = read_type_base(tokens, i);
    // `T?`: an optional type
    if !base.is_empty() && kind(tokens, next) == Some(&T::Question) {
        return (format!("{}?", base), next + 1);
    }
    (base, next)
}

fn read_type_base(tokens: &[Token], i: usize) -> (String, usize) {
    match kind(tokens, i) {
        Some(T::Function) => {
            let Some(close) = (kind(tokens, i + 1) == Some(&T::LParen)).then(|| closing(tokens, i + 1)).flatten() else {
                return (String::new(), i);
            };
            let mut params = Vec::new();
            let mut j = i + 2;
            while j < close {
                let (t, next) = read_type(tokens, j);
                if next == j {
                    break;
                }
                params.push(t);
                j = next;
                if kind(tokens, j) == Some(&T::Comma) {
                    j += 1;
                }
            }
            let (ret, after) = if kind(tokens, close + 1) == Some(&T::Arrow) {
                read_type(tokens, close + 2)
            } else {
                ("void".to_string(), close + 1)
            };
            (format!("fn({}) -> {}", params.join(", "), ret), after)
        }
        Some(T::TypeArray) if kind(tokens, i + 1) == Some(&T::Lt) => {
            let (inner, after) = read_type(tokens, i + 2);
            let after = if kind(tokens, after) == Some(&T::Gt) { after + 1 } else { after };
            (format!("array<{}>", inner), after)
        }
        Some(T::Ident(n)) if n == "map" && kind(tokens, i + 1) == Some(&T::Lt) => {
            let (key, after) = read_type(tokens, i + 2);
            let (value, after) = if kind(tokens, after) == Some(&T::Comma) {
                read_type(tokens, after + 1)
            } else {
                (String::new(), after)
            };
            let after = if kind(tokens, after) == Some(&T::Gt) { after + 1 } else { after };
            (format!("map<{}, {}>", key, value), after)
        }
        Some(T::Ident(n)) if n == "task" && kind(tokens, i + 1) == Some(&T::Lt) => {
            let (inner, after) = read_type(tokens, i + 2);
            let after = if kind(tokens, after) == Some(&T::Gt) { after + 1 } else { after };
            (format!("task<{}>", inner), after)
        }
        Some(T::Ident(n)) => (n.clone(), i + 1),
        Some(T::TypeInt) => ("int".into(), i + 1),
        Some(T::TypeFloat) => ("float".into(), i + 1),
        Some(T::TypeBool) => ("bool".into(), i + 1),
        Some(T::TypeString) => ("string".into(), i + 1),
        Some(T::TypeVoid) => ("void".into(), i + 1),
        _ => (String::new(), i),
    }
}

/// the params `( a: T, b: U )` whose `(` is at `open`: (name, type) pairs,
/// and the index of the closing `)`
pub fn read_params(tokens: &[Token], open: usize) -> Option<(Vec<(String, String)>, usize)> {
    let close = closing(tokens, open)?;
    let mut params = Vec::new();
    let mut i = open + 1;
    while i < close {
        if let (Some(T::Ident(name)), Some(T::Colon)) = (kind(tokens, i), kind(tokens, i + 1)) {
            let (ty, next) = read_type(tokens, i + 2);
            params.push((name.clone(), ty));
            i = next.max(i + 1);
        } else {
            i += 1;
        }
    }
    Some((params, close))
}

/// `-> R` after the `)` at `close`, or `void`
pub fn return_after(tokens: &[Token], close: usize) -> String {
    if kind(tokens, close + 1) == Some(&T::Arrow) {
        read_type(tokens, close + 2).0
    } else {
        "void".to_string()
    }
}

/// every named function declared in the document, in order
pub fn declared(tokens: &[Token]) -> Vec<Signature> {
    let mut out = Vec::new();
    for i in 0..tokens.len() {
        if let (Some(T::Function), Some(T::Ident(name)), Some(T::LParen)) = (kind(tokens, i), kind(tokens, i + 1), kind(tokens, i + 2)) {
            if let Some((params, close)) = read_params(tokens, i + 2) {
                out.push(Signature { name: name.clone(), params, ret: return_after(tokens, close) });
            }
        }
    }
    out
}

/// names bound to a function type or a lambda: `NAME: function(..) -> R` (a
/// param, a typed let) and `let NAME = function(..) -> R { .. }`
pub fn bound_values(tokens: &[Token]) -> Vec<Signature> {
    let mut out = Vec::new();
    for i in 0..tokens.len() {
        // `NAME: function(A, B) -> R`
        if let (Some(T::Ident(name)), Some(T::Colon), Some(T::Function), Some(T::LParen)) =
            (kind(tokens, i), kind(tokens, i + 1), kind(tokens, i + 2), kind(tokens, i + 3))
        {
            let (ty, _) = read_type(tokens, i + 2);
            if let Some(sig) = from_function_type(name, &ty) {
                out.push(sig);
            }
        }
        // `let NAME = function(x: T) -> R { .. }`
        if let (Some(T::Let | T::Const), Some(T::Ident(name)), Some(T::Eq), Some(T::Function), Some(T::LParen)) =
            (kind(tokens, i), kind(tokens, i + 1), kind(tokens, i + 2), kind(tokens, i + 3), kind(tokens, i + 4))
        {
            if let Some((params, close)) = read_params(tokens, i + 4) {
                out.push(Signature { name: name.clone(), params, ret: return_after(tokens, close) });
            }
        }
    }
    out
}

/// a function TYPE's text (`function(int, string) -> bool`) as a signature with
/// unnamed params
fn from_function_type(name: &str, ty: &str) -> Option<Signature> {
    let inner = ty.strip_prefix("fn(")?;
    // the param list ends at the first `) -> ` that isn't inside `<...>` or
    // another `function(`. a tiny depth scan is enough for what read_type writes
    let mut depth: i32 = 0;
    let mut split = None;
    let mut prev = ' ';
    for (i, c) in inner.char_indices() {
        match c {
            '(' | '<' => depth += 1,
            // the `>` of `->` is an arrow, not the end of a `<...>`
            '>' if prev == '-' => {}
            '>' => depth -= 1,
            ')' if depth == 0 => {
                split = Some(i);
                break;
            }
            ')' => depth -= 1,
            _ => {}
        }
        prev = c;
    }
    let split = split?;
    let params_text = &inner[..split];
    let ret = inner[split..].strip_prefix(") -> ")?.to_string();
    let params = split_top(params_text)
        .into_iter()
        .filter(|p| !p.is_empty())
        .map(|p| (String::new(), p))
        .collect();
    Some(Signature { name: name.to_string(), params, ret })
}

/// splits on commas that aren't inside `<>` or `()`
fn split_top(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut current = String::new();
    let mut prev = ' ';
    for c in text.chars() {
        match c {
            '<' | '(' => depth += 1,
            '>' if prev == '-' => {}
            '>' | ')' => depth -= 1,
            ',' if depth == 0 => {
                out.push(current.trim().to_string());
                current.clear();
                prev = c;
                continue;
            }
            _ => {}
        }
        current.push(c);
        prev = c;
    }
    out.push(current.trim().to_string());
    out
}

/// every param of every function (named or lambda) in the document, with
/// its type text. params are offered as completions inside their bodies
pub fn all_params(tokens: &[Token]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for i in 0..tokens.len() {
        if kind(tokens, i) != Some(&T::Function) {
            continue;
        }
        let open = match (kind(tokens, i + 1), kind(tokens, i + 2)) {
            (Some(T::Ident(_)), Some(T::LParen)) => i + 2,
            (Some(T::LParen), _) => i + 1,
            _ => continue,
        };
        if let Some((params, _)) = read_params(tokens, open) {
            out.extend(params);
        }
    }
    out
}

/// the call that the cursor is inside: the callee's name and the index of the
/// arg the cursor is on. `at` is the number of tokens before the cursor.
/// stops at a brace, so a call never leaks into a neighbouring block
pub fn call_at(tokens: &[Token], at: usize) -> Option<(String, usize)> {
    let (callee, active) = call_site(tokens, at)?;
    match kind(tokens, callee)? {
        T::Ident(name) => Some((name.clone(), active)),
        _ => None,
    }
}

/// like `call_at`, but gives the index of the callee's token (the name right
/// before the `(`), so a method call can look at the receiver before its dot
pub fn call_site(tokens: &[Token], at: usize) -> Option<(usize, usize)> {
    let mut depth = 0usize;
    let mut commas = 0usize;
    // a balanced `{ ... }` that sits inside the call (a lambda passed as an
    // arg) is skipped whole. the first unmatched `{` ends the search
    let mut braces = 0usize;
    for j in (0..at.min(tokens.len())).rev() {
        match tokens[j].kind {
            T::RBrace => braces += 1,
            T::LBrace if braces > 0 => braces -= 1,
            T::LBrace => return None,
            _ if braces > 0 => {}
            T::RParen => depth += 1,
            T::LParen => {
                if depth == 0 {
                    return Some((j.checked_sub(1)?, commas));
                }
                depth -= 1;
            }
            T::Comma if depth == 0 => commas += 1,
            _ => {}
        }
    }
    None
}

/// the signature of the call around the cursor, and the active arg.
/// named functions win over values of the same name
pub fn signature_help(tokens: &[Token], at: usize) -> Option<(Signature, usize)> {
    let (name, active) = call_at(tokens, at)?;
    let sig = declared(tokens)
        .into_iter()
        .chain(bound_values(tokens))
        .find(|s| s.name == name)?;
    Some((sig, active))
}

/// the label of every callable the document declares, by name, for completion details
pub fn label_by_name(tokens: &[Token]) -> Vec<(String, String)> {
    declared(tokens)
        .into_iter()
        .chain(bound_values(tokens))
        .map(|s| (s.name.clone(), s.label()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(src: &str) -> Vec<Token> {
        spg_lexer::Lexer::new(src).tokenize().unwrap()
    }

    #[test]
    fn named_function_label_and_params() {
        let t = toks("function dobro(x: int) -> int { return x * 2 }");
        let s = declared(&t);
        assert_eq!(s[0].label(), "dobro(x: int) -> int");
    }

    #[test]
    fn function_type_parameter_is_a_callable() {
        let t = toks("function aplica(f: function(int, string) -> bool, n: int) -> int { return 0 }");
        let vals = bound_values(&t);
        assert_eq!(vals[0].name, "f");
        assert_eq!(vals[0].label(), "f(int, string) -> bool");
    }

    #[test]
    fn let_bound_lambda_keeps_its_parameter_names() {
        let t = toks("let s = function(a: int, b: int) -> int { return a + b }");
        assert_eq!(bound_values(&t)[0].label(), "s(a: int, b: int) -> int");
    }

    #[test]
    fn active_argument_counts_top_level_commas_only() {
        // `f(a, g(b, c), |)` -- the cursor is the third arg of f
        let t = toks("print(f(a, g(b, c), 1))");
        let at = t.iter().position(|x| x.kind == T::Int(1)).unwrap();
        assert_eq!(call_at(&t, at), Some(("f".to_string(), 2)));
    }

    #[test]
    fn lambda_parameters_are_offered_for_completion() {
        let t = toks("let k = function(value: int) -> int { return value }");
        assert!(all_params(&t).iter().any(|(n, ty)| n == "value" && ty == "int"));
    }

    #[test]
    fn call_does_not_leak_out_of_a_block() {
        let t = toks("function f() -> void { print(1) }\nlet z = 2");
        let at = t.len();
        assert_eq!(call_at(&t, at), None);
    }
}

#[cfg(test)]
mod optional_tests {
    use super::*;

    #[test]
    fn optional_types_are_read_whole() {
        let t = spg_lexer::Lexer::new("function f(a: int?, b: array<string?>, g: function(int?) -> int?) -> int? { return none }")
            .tokenize()
            .unwrap();
        let sigs = declared(&t);
        assert_eq!(sigs[0].label(), "f(a: int?, b: array<string?>, g: function(int?) -> int?) -> int?");
    }
}
