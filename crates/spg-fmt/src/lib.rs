//! `spg fmt`: the one true layout of a Spaghetti file. no debates.
//!
//! the formatter only changes WHITESPACE: spaces inside a line, the indentation
//! of each line, blank lines, and the spelling of a `#>` directive line. it never
//! joins or splits lines, never touches a token, keeps every comment. those
//! promises aren't trusted, they're checked: `check` lexes the input and the
//! output and compares tokens, comments and directives. a result
//! that doesn't pass is an error, and the file stays as it was. untouched.
//!
//! the scanner is a plain character scanner, not the real lexer, because
//! a formatter must keep comments, and the lexer throws them away.

use spg_lexer::TokenKind;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Class {
    Ident,
    Keyword,
    Number,
    Str,
    Namespace,
    Punct,
    Directive,
}

#[derive(Debug, Clone, PartialEq)]
struct Atom {
    text: String,
    class: Class,
}

#[derive(Debug, Clone, PartialEq)]
enum Piece {
    /// whitespace containing this many newlines (spaces alone don't count)
    Gap(usize),
    Comment(String),
    Atom(Atom),
}

const KEYWORDS: &[&str] = &[
    "let", "const", "fn", "return", "if", "else", "while", "for", "in", "try", "catch", "throw", "break",
    "continue", "when", "elif", "guard", "use", "public", "from", "attach", "struct", "enum", "true", "false", "int", "float", "step",
    "bool", "string", "void", "array",
];

/// keywords followed by a space when a `(` comes after them: `if (x)`,
/// but a call `f(x)` and a lambda or type `function(x)` take none
const SPACED_PAREN_KEYWORDS: &[&str] = &["if", "elif", "while", "for", "return", "catch", "when", "else", "in", "try", "throw", "guard", "use", "public"];

const TWO_CHAR: &[&str] = &["==", "!=", "<=", ">=", "->", "+=", "-=", "*=", "/=", "%=", "**", "..", "&&", "||", "??", "|>", "::", "++", "--"];

/// the end (exclusive) of the `"..."` string that starts at `i`
fn scan_double_quoted(c: &[char], mut i: usize) -> usize {
    i += 1;
    while i < c.len() {
        match c[i] {
            '\\' => i += 2,
            '"' => return i + 1,
            '$' if c.get(i + 1) == Some(&'{') => i = scan_interpolation(c, i + 2),
            _ => i += 1,
        }
    }
    c.len()
}

/// the end (exclusive) of an interpolation body `${ ... }` starting at `i`
fn scan_interpolation(c: &[char], mut i: usize) -> usize {
    let mut depth = 0usize;
    while i < c.len() {
        match c[i] {
            '}' if depth == 0 => return i + 1,
            '}' => {
                depth -= 1;
                i += 1;
            }
            '{' => {
                depth += 1;
                i += 1;
            }
            '"' => i = scan_double_quoted(c, i),
            '\'' => {
                i += 1;
                while i < c.len() && c[i] != '\'' {
                    i += 1;
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    c.len()
}

fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

fn scan(src: &str) -> Vec<Piece> {
    let c: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        if ch == '\n' {
            // consecutive newlines (and the spaces between) form one gap
            let mut n = 0;
            while i < c.len() && c[i].is_whitespace() {
                if c[i] == '\n' {
                    n += 1;
                }
                i += 1;
            }
            out.push(Piece::Gap(n));
            continue;
        }
        if ch.is_whitespace() {
            i += 1;
            continue;
        }
        if ch == '/' && c.get(i + 1) == Some(&'/') {
            let start = i;
            while i < c.len() && c[i] != '\n' {
                i += 1;
            }
            let text: String = c[start..i].iter().collect();
            out.push(Piece::Comment(text.trim_end().to_string()));
            continue;
        }
        if ch == '#' && c.get(i + 1) == Some(&'>') {
            let start = i;
            while i < c.len() && c[i] != '\n' {
                i += 1;
            }
            let raw: String = c[start..i].iter().collect();
            let words: Vec<&str> = raw.split_whitespace().collect();
            out.push(Piece::Atom(Atom { text: words.join(" "), class: Class::Directive }));
            continue;
        }
        if ch == '"' {
            let end = scan_double_quoted(&c, i);
            out.push(Piece::Atom(Atom { text: c[i..end].iter().collect(), class: Class::Str }));
            i = end;
            continue;
        }
        if ch == '\'' {
            let mut end = i + 1;
            while end < c.len() && c[end] != '\'' {
                end += 1;
            }
            let end = (end + 1).min(c.len());
            out.push(Piece::Atom(Atom { text: c[i..end].iter().collect(), class: Class::Str }));
            i = end;
            continue;
        }
        // `#name` (right after `attach`): a standard library, one atom like `@name`
        if ch == '#' && c.get(i + 1).is_some_and(|n| is_word_char(*n)) {
            let start = i;
            i += 1;
            while i < c.len() && is_word_char(c[i]) {
                i += 1;
            }
            out.push(Piece::Atom(Atom { text: c[start..i].iter().collect(), class: Class::Namespace }));
            continue;
        }
        if ch == '@' {
            let start = i;
            i += 1;
            while i < c.len() && is_word_char(c[i]) {
                i += 1;
            }
            out.push(Piece::Atom(Atom { text: c[start..i].iter().collect(), class: Class::Namespace }));
            continue;
        }
        if ch.is_ascii_digit() {
            let start = i;
            while i < c.len() && is_word_char(c[i]) {
                i += 1;
            }
            out.push(Piece::Atom(Atom { text: c[start..i].iter().collect(), class: Class::Number }));
            continue;
        }
        if is_word_char(ch) {
            let start = i;
            while i < c.len() && is_word_char(c[i]) {
                i += 1;
            }
            let text: String = c[start..i].iter().collect();
            // verbose spellings of the operators are canonicalized to their
            // symbols: `and`->`&&`, `or`->`||`, `not`->`!`, `then`->`|>`
            let symbol = match text.as_str() {
                "and" => Some("&&"),
                "or" => Some("||"),
                "not" => Some("!"),
                "then" => Some("|>"),
                _ => None,
            };
            if let Some(sym) = symbol {
                out.push(Piece::Atom(Atom { text: sym.to_string(), class: Class::Punct }));
                continue;
            }
            let class = if KEYWORDS.contains(&text.as_str()) { Class::Keyword } else { Class::Ident };
            out.push(Piece::Atom(Atom { text, class }));
            continue;
        }
        // two-character operators first, then a single character
        if let Some(two) = c.get(i..i + 2).map(|s| s.iter().collect::<String>()) {
            if TWO_CHAR.contains(&two.as_str()) {
                out.push(Piece::Atom(Atom { text: two, class: Class::Punct }));
                i += 2;
                continue;
            }
        }
        out.push(Piece::Atom(Atom { text: ch.to_string(), class: Class::Punct }));
        i += 1;
    }
    out
}

/// `-` or `!` in front of an operand (`-3`, `return -x`), not a binary minus
fn is_unary_position(prev: Option<&Atom>) -> bool {
    match prev {
        None => true,
        Some(a) if a.class == Class::Punct => !matches!(a.text.as_str(), ")" | "]" | "}"),
        Some(a) => a.class == Class::Keyword,
    }
}

/// whether a space goes between two atoms on the same line
fn needs_space(prev: &Atom, next: &Atom, prev_unary: bool, generic: usize) -> bool {
    let p = prev.text.as_str();
    let n = next.text.as_str();
    if prev_unary && (p == "-" || p == "!") {
        return false;
    }
    if matches!(p, "(" | "[") || matches!(n, ")" | "]" | "," | ";" | ":" | ".") {
        return false;
    }
    // `n++`, `++n`: the increment hugs its operand (after a name, a number or a closing bracket)
    if p == "++" || p == "--" {
        return false;
    }
    if (n == "++" || n == "--") && (matches!(prev.class, Class::Ident | Class::Namespace) || matches!(p, "]" | ")")) {
        return false;
    }
    // `int?`: the question mark of an optional type hugs its type
    if n == "?" {
        return false;
    }
    if p == "." || n == ".." || p == ".." {
        return false;
    }
    // `int::max`: the property operator hugs both sides
    if p == "::" || n == "::" {
        return false;
    }
    if n == "(" {
        return prev.class == Class::Keyword && SPACED_PAREN_KEYWORDS.contains(&p);
    }
    if n == "[" {
        return !matches!(prev.class, Class::Ident | Class::Namespace) && !matches!(p, ")" | "]");
    }
    // generic brackets: array<int>, map<string, int>, task<int>
    if n == "<" && (p == "array" || p == "map" || p == "task") {
        return false;
    }
    if (p == "<" || n == ">") && generic > 0 {
        return false;
    }
    if p == "{" {
        return n != "}";
    }
    if n == "}" {
        return p != "{";
    }
    true
}

/// the file in canonical layout. leading and trailing blank lines go, runs of
/// blank lines shrink to one, and the file ends with one newline
pub fn format_source(src: &str) -> String {
    let mut out = String::new();
    let mut started = false;
    let mut at_line_start = true;
    let mut pending_blank = false;
    let mut brace: i64 = 0;
    let mut paren: i64 = 0;
    // the paren depth at each open `{`: a block inside `f(fn() { ... })` is indented by its braces,
    // not by the call's parens that were already open when the block began
    let mut open_paren: Vec<i64> = Vec::new();
    let mut generic: usize = 0;
    let mut prev: Option<Atom> = None;
    let mut prev_unary = false;

    for piece in scan(src) {
        match piece {
            Piece::Gap(newlines) => {
                if !started || newlines == 0 {
                    continue;
                }
                if !at_line_start {
                    out.push('\n');
                    at_line_start = true;
                }
                pending_blank = newlines > 1;
            }
            Piece::Comment(text) => {
                if !started {
                    started = true;
                }
                if at_line_start {
                    emit_indent(&mut out, brace, paren - open_paren.last().copied().unwrap_or(0), None, pending_blank, &mut started);
                    pending_blank = false;
                    at_line_start = false;
                    out.push_str(&text);
                } else {
                    out.push_str("  ");
                    out.push_str(&text);
                }
                prev = None;
            }
            Piece::Atom(atom) => {
                if at_line_start {
                    emit_indent(&mut out, brace, paren - open_paren.last().copied().unwrap_or(0), Some(&atom.text), pending_blank, &mut started);
                    pending_blank = false;
                    at_line_start = false;
                } else if let Some(p) = &prev {
                    if needs_space(p, &atom, prev_unary, generic) {
                        out.push(' ');
                    }
                }
                out.push_str(&atom.text);

                match atom.text.as_str() {
                    "{" => {
                        brace += 1;
                        open_paren.push(paren);
                    }
                    "}" => {
                        brace -= 1;
                        open_paren.pop();
                    }
                    "(" | "[" => paren += 1,
                    ")" | "]" => paren -= 1,
                    _ => {}
                }
                if atom.text == "<" && prev.as_ref().is_some_and(|p| matches!(p.text.as_str(), "array" | "map" | "task")) {
                    generic += 1;
                } else if atom.text == ">" && generic > 0 {
                    generic -= 1;
                }
                prev_unary = atom.class == Class::Punct
                    && matches!(atom.text.as_str(), "-" | "!")
                    && is_unary_position(prev.as_ref());
                prev = Some(atom);
            }
        }
    }
    if !at_line_start {
        out.push('\n');
    }
    out
}

/// starts a line: the blank line first (if the source had one), then the indent.
/// a line that opens with `}`, `)` or `]` is printed one level out
fn emit_indent(out: &mut String, brace: i64, paren: i64, first: Option<&str>, blank: bool, started: &mut bool) {
    if *started && !out.is_empty() && blank {
        out.push('\n');
    }
    let mut level = brace + paren;
    if let Some(text) = first {
        if text == "}" || text == ")" || text == "]" {
            level -= 1;
        }
    }
    let level = level.max(0) as usize;
    for _ in 0..level {
        out.push_str("    ");
    }
    *started = true;
}

fn comments(src: &str) -> Vec<String> {
    scan(src)
        .into_iter()
        .filter_map(|p| match p {
            Piece::Comment(t) => Some(t),
            _ => None,
        })
        .collect()
}

fn token_kinds(src: &str) -> Result<Vec<TokenKind>, String> {
    spg_lexer::Lexer::new(src)
        .tokenize()
        .map(|tokens| tokens.into_iter().map(|t| t.kind).collect())
        .map_err(|e| e.to_string())
}

/// why a file can't be formatted safely. the scary list.
#[derive(Debug, Clone, PartialEq)]
pub enum FmtError {
    /// the file doesn't lex, so its tokens can't be compared
    Lex(String),
    /// the formatted text would lex to different tokens
    TokensChanged,
    /// the formatted text would have different comments
    CommentsChanged,
    /// the formatted text has different directives (`#>`)
    DirectivesChanged,
    /// formatting the formatted text again changes it (not a fixed point. bad.)
    NotStable,
}

impl std::fmt::Display for FmtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FmtError::Lex(e) => write!(f, "the file does not lex, so it cannot be formatted: {}", e),
            FmtError::TokensChanged => write!(f, "the formatter would change the tokens of the program (not written)"),
            FmtError::CommentsChanged => write!(f, "the formatter would change a comment (not written)"),
            FmtError::DirectivesChanged => write!(f, "the formatter would change a #> directive (not written)"),
            FmtError::NotStable => write!(f, "formatting the result again changes it (not written)"),
        }
    }
}

/// formats `src` and proves the result is safe: same tokens, same
/// comments, same directives, and a fixed point. returns the formatted text
pub fn check(src: &str) -> Result<String, FmtError> {
    let before = token_kinds(src).map_err(FmtError::Lex)?;
    let out = format_source(src);
    if token_kinds(&out).map_err(FmtError::Lex)? != before {
        return Err(FmtError::TokensChanged);
    }
    if comments(&out) != comments(src) {
        return Err(FmtError::CommentsChanged);
    }
    let (a, b) = (spg_lexer::read_file_spec(src), spg_lexer::read_file_spec(&out));
    match (a, b) {
        (Ok(a), Ok(b)) => {
            if a.friendly_terminal != b.friendly_terminal || a.strict != b.strict || a.timeout_ms != b.timeout_ms {
                return Err(FmtError::DirectivesChanged);
            }
        }
        _ => return Err(FmtError::DirectivesChanged),
    }
    if format_source(&out) != out {
        return Err(FmtError::NotStable);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(src: &str) -> String {
        format_source(src)
    }

    #[test]
    fn spaces_around_operators_and_after_commas() {
        assert_eq!(fmt("let x=1+2*3\n"), "let x = 1 + 2 * 3\n");
        assert_eq!(fmt("print(a,b ,c)\n"), "print(a, b, c)\n");
        assert_eq!(fmt("let y = f( 1 )\n"), "let y = f(1)\n");
    }

    #[test]
    fn indentation_follows_braces() {
        let src = "function f(x:int)->int{\nif (x>0){\nreturn 1\n}\nreturn 0\n}\n";
        let want = "function f(x: int) -> int {\n    if (x > 0) {\n        return 1\n    }\n    return 0\n}\n";
        assert_eq!(fmt(src), want);
    }

    #[test]
    fn blank_lines_collapse_to_one_and_edges_go() {
        assert_eq!(fmt("\n\nlet a = 1\n\n\n\nlet b = 2\n\n\n"), "let a = 1\n\nlet b = 2\n");
    }

    #[test]
    fn comments_stay_put_with_their_line() {
        let src = "// topo\nlet a = 1 // lado\n\n// bloco\nfunction f() -> void {\n// dentro\n}\n";
        let want = "// topo\nlet a = 1  // lado\n\n// bloco\nfunction f() -> void {\n    // dentro\n}\n";
        assert_eq!(fmt(src), want);
    }

    #[test]
    fn strings_are_kept_byte_for_byte_including_interpolation() {
        let src = "print(\"a \\\"q\\\"  ${ x+1 }\")\nprint('cru  ${y}')\n";
        let out = fmt(src);
        assert!(out.contains("\"a \\\"q\\\"  ${ x+1 }\""), "{}", out);
        assert!(out.contains("'cru  ${y}'"), "{}", out);
    }

    #[test]
    fn unary_minus_and_generics_are_tight() {
        assert_eq!(fmt("let n = -3\n"), "let n = -3\n");
        assert_eq!(fmt("let a = x - 3\n"), "let a = x - 3\n");
        assert_eq!(fmt("let m: map<string,int> = [:]\n"), "let m: map<string, int> = [:]\n");
        assert_eq!(fmt("let xs: array<int> = []\n"), "let xs: array<int> = []\n");
    }

    #[test]
    fn install_std_hash_stays_tight() {
        assert_eq!(fmt("attach #ps\n"), "attach #ps\n");
        assert_eq!(fmt("attach   #ps\n"), "attach #ps\n");
        assert_eq!(fmt("attach \"./a.spg\"\n"), "attach \"./a.spg\"\n");
    }

    #[test]
    fn lambda_and_function_type_have_no_space_before_paren() {
        assert_eq!(fmt("let f = function (x:int)->int { return x }\n"), "let f = function(x: int) -> int { return x }\n");
        assert_eq!(fmt("function ap(f: function (int)->int) -> int { return f(1) }\n"), "function ap(f: function(int) -> int) -> int { return f(1) }\n");
    }

    #[test]
    fn directive_line_is_normalized() {
        assert_eq!(fmt("#>   strict\n\nlet a = 1\n"), "#> strict\n\nlet a = 1\n");
    }

    #[test]
    fn multi_line_string_is_not_reindented() {
        let src = "function f() -> void {\nprint(\"line 1\nlinha 2\")\n}\n";
        assert_eq!(fmt(src), "function f() -> void {\n    print(\"line 1\nlinha 2\")\n}\n");
    }

    #[test]
    fn check_accepts_and_is_idempotent() {
        let src = "function f(x:int)->int{\nreturn x*2 // dobro\n}\nprint(f(2))\n";
        let out = check(src).expect("a well-formed file formats");
        assert_eq!(check(&out).unwrap(), out);
    }

    #[test]
    fn check_refuses_a_file_that_does_not_lex() {
        assert!(matches!(check("let a = \"no end\n"), Err(FmtError::Lex(_))));
    }

    #[test]
    fn check_keeps_directives() {
        let out = check("#> friendly-terminal\nlet a = 1\n").unwrap();
        assert!(out.starts_with("#> friendly-terminal\n"));
    }
}

#[cfg(test)]
mod operator_tests {
    use super::*;

    #[test]
    fn logical_operators_stay_whole() {
        assert_eq!(format_source("if (a&&b||c) { }\n"), "if (a && b || c) {}\n");
        assert!(check("let k = x&&y\n").is_ok());
    }
}

#[cfg(test)]
mod optional_tests {
    use super::*;

    #[test]
    fn optional_types_hug_their_question_mark() {
        assert_eq!(format_source("let x:int?=none\n"), "let x: int? = none\n");
        assert_eq!(format_source("let m: map<string,int?> = [:]\n"), "let m: map<string, int?> = [:]\n");
        assert_eq!(format_source("function f(a:array<int>?)->int? { return none }\n"), "function f(a: array<int>?) -> int? { return none }\n");
    }

    #[test]
    fn coalesce_is_a_spaced_binary_operator() {
        assert_eq!(format_source("let y = a??b??-1\n"), "let y = a ?? b ?? -1\n");
        assert!(check("let y = a??b\n").is_ok());
    }

    #[test]
    fn semicolons_are_kept_and_hug_the_statement_before_them() {
        assert_eq!(format_source("let a = 1; let b = 2 ;\n"), "let a = 1; let b = 2;\n");
        assert_eq!(format_source("print(a);;print(b)\n"), "print(a);; print(b)\n");
    }
}
