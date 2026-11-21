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
    /// a line comment `// ...`: it ends its line
    Comment(String),
    /// a block comment `/{ ... }/`: it may span lines and does not end the line
    Block(String),
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
        if ch == '/' && c.get(i + 1) == Some(&'{') {
            // `/{ ... }/` ends at the first `}/` (no nesting, as the lexer reads it)
            let start = i;
            let mut j = i + 2;
            while j + 1 < c.len() && !(c[j] == '}' && c[j + 1] == '/') {
                j += 1;
            }
            if j + 1 < c.len() {
                let text: String = c[start..j + 2].iter().collect();
                out.push(Piece::Block(text));
                i = j + 2;
                continue;
            }
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
    // the atom before `prev` is `fn`: `prev` is a function's name, so a `<` after it opens type parameters
    let mut fn_name_prev = false;

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
            Piece::Comment(text) | Piece::Block(text) => {
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
                    // `fn add<T: Numeric>`: the type parameters hug the name
                    let hug_type_params = fn_name_prev && atom.text == "<";
                    if !hug_type_params && needs_space(p, &atom, prev_unary, generic) {
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
                if atom.text == "<" && (fn_name_prev || prev.as_ref().is_some_and(|p| matches!(p.text.as_str(), "array" | "map" | "task"))) {
                    generic += 1;
                } else if atom.text == ">" && generic > 0 {
                    generic -= 1;
                }
                prev_unary = atom.class == Class::Punct
                    && matches!(atom.text.as_str(), "-" | "!")
                    && is_unary_position(prev.as_ref());
                fn_name_prev = atom.class == Class::Ident && prev.as_ref().is_some_and(|p| p.text == "fn");
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
            Piece::Comment(t) | Piece::Block(t) => Some(t),
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
    /// the compact text would parse to a different program
    AstChanged,
    /// the encoded text would not run the same program with the names swapped back
    EncodeChanged,
}

impl std::fmt::Display for FmtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FmtError::Lex(e) => write!(f, "the file does not lex, so it cannot be formatted: {}", e),
            FmtError::TokensChanged => write!(f, "the formatter would change the tokens of the program (not written)"),
            FmtError::CommentsChanged => write!(f, "the formatter would change a comment (not written)"),
            FmtError::DirectivesChanged => write!(f, "the formatter would change a #> directive (not written)"),
            FmtError::NotStable => write!(f, "formatting the result again changes it (not written)"),
            FmtError::AstChanged => write!(f, "the compact text would parse to a different program (not written)"),
            FmtError::EncodeChanged => write!(f, "the encoded text would not run the same program (not written)"),
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

// ---- compact layouts: `--min` (keeps comments) and `--min-nc` (drops them) ----

mod encode;
pub use encode::encode_sources;

/// the layout `spg fmt` writes. canonical is the default; compact drops the
/// indentation, the blank lines and every space that isn't needed
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Layout {
    Canonical,
    /// `keep_comments` is `--min`; without it (`--min-nc`) the comments go too
    Compact { keep_comments: bool },
}

/// `spg fmt` in the given layout, proved safe the same way `check` proves the canonical one
pub fn check_layout(src: &str, layout: Layout) -> Result<String, FmtError> {
    match layout {
        Layout::Canonical => check(src),
        Layout::Compact { keep_comments } => check_compact(src, keep_comments),
    }
}

fn is_word_start(s: &str) -> bool {
    s.chars().next().is_some_and(|c| is_word_char(c) || matches!(c, '@' | '#' | '"' | '\''))
}

fn is_word_end(s: &str) -> bool {
    s.chars().last().is_some_and(|c| is_word_char(c) || matches!(c, '"' | '\''))
}

/// a space goes between two atoms of the compact layout only when, without it,
/// they would lex as one token: two words, or two operators that fuse (`- -x`)
fn compact_space(prev: &Atom, next: &Atom) -> bool {
    if is_word_end(&prev.text) && is_word_start(&next.text) {
        return true;
    }
    if prev.class == Class::Punct && next.class == Class::Punct {
        if let (Some(a), Some(b)) = (prev.text.chars().last(), next.text.chars().next()) {
            let pair: String = [a, b].iter().collect();
            return TWO_CHAR.contains(&pair.as_str());
        }
    }
    false
}

/// what a `(`, `[` or `{` opens. a `Decl` body (a struct, an enum, the arms of a
/// `when`, a struct literal) takes no `;`: its parser reads the lines by their
/// position, not by a separator
#[derive(Debug, Clone, Copy, PartialEq)]
enum Open {
    Paren { when: bool },
    Bracket,
    Block,
    Decl,
}

/// the line break after an atom becomes `;` when the parser reads it as the end of
/// a statement. it does not, after an operator or `,`, inside `(`, `[` or a `Decl`
/// body, before a `)`, `]` or `}`, before a `{`, or before `else`, `elif` and `catch`
fn compact_semi(last_cont: bool, next: Option<&Atom>, stack: &[Open]) -> bool {
    if last_cont || matches!(stack.last(), Some(Open::Paren { .. }) | Some(Open::Bracket) | Some(Open::Decl)) {
        return false;
    }
    match next {
        None => false,
        Some(a) => match a.class {
            // `-` and `(` and `[` start a statement after a newline (the parser stops there);
            // any other punctuation is a continuation or a closing bracket
            Class::Punct => matches!(a.text.as_str(), "(" | "[" | "-" | "!" | "++" | "--"),
            Class::Keyword => !matches!(a.text.as_str(), "else" | "elif" | "catch"),
            _ => true,
        },
    }
}

/// the first atom after `from`, skipping gaps and comments
fn next_atom(pieces: &[Piece], from: usize) -> Option<&Atom> {
    pieces[from + 1..].iter().find_map(|p| match p {
        Piece::Atom(a) => Some(a),
        _ => None,
    })
}

/// the compact text of `src`: no indentation, no blank lines, no space where it
/// isn't needed. statement ends become `;`. with `keep_comments` false, comments go
pub fn format_compact(src: &str, keep_comments: bool) -> String {
    format_compact_pieces(scan(src), keep_comments)
}

/// the compact layout of already scanned pieces (`--enc` renames names in them first)
fn format_compact_pieces(pieces: Vec<Piece>, keep_comments: bool) -> String {
    let mut out = String::new();
    // the last atom written, and whether it expects more (an operator, a `,`, `fn`...)
    let mut last: Option<Atom> = None;
    let mut last_cont = false;
    let mut stack: Vec<Open> = Vec::new();
    let mut generic: usize = 0;
    let mut fn_name_prev = false;
    // `struct` or `enum` (or `when`) was just read: its `{` is a Decl body
    let mut pending_decl = false;
    let mut pending_when = false;
    // the atom just written closed a `when (...)`: the `{` after it is its arms
    let mut closed_when = false;
    // a line break was seen since the last atom
    let mut newline = false;
    // a line comment or `#>` directive ends its line: the next thing starts a new one
    let mut need_eol = false;

    for (idx, piece) in pieces.iter().enumerate() {
        match piece {
            Piece::Gap(_) => newline = true,
            Piece::Comment(text) => {
                if !keep_comments {
                    continue;
                }
                if need_eol {
                    out.push('\n');
                    last = None;
                } else if last.is_some() {
                    if newline {
                        if compact_semi(last_cont, next_atom(&pieces, idx), &stack) {
                            out.push(';');
                        }
                    } else if !out.ends_with(';') {
                        out.push(' ');
                    }
                }
                out.push_str(text);
                need_eol = true;
                newline = false;
            }
            Piece::Block(text) => {
                if !keep_comments {
                    continue;
                }
                // it sits between two things on the line: a space keeps it apart from both
                if last.is_some() && !out.ends_with('\n') && !out.ends_with(';') {
                    out.push(' ');
                }
                out.push_str(text);
                out.push(' ');
            }
            Piece::Atom(a) if a.class == Class::Directive => {
                // a directive is a whole line of its own
                if need_eol || last.is_some() {
                    out.push('\n');
                }
                out.push_str(&a.text);
                need_eol = true;
                last = None;
                newline = false;
            }
            Piece::Atom(a) => {
                if need_eol {
                    out.push('\n');
                    need_eol = false;
                    last = None;
                    newline = false;
                }
                if let Some(prev) = &last {
                    if newline && compact_semi(last_cont, Some(a), &stack) {
                        out.push(';');
                    } else if compact_space(prev, a) {
                        out.push(' ');
                    }
                }
                let was_closed_when = closed_when;
                closed_when = false;
                let closing = a.text == ">" && generic > 0;
                let opening = a.text == "<"
                    && (fn_name_prev || last.as_ref().is_some_and(|p| matches!(p.text.as_str(), "array" | "map" | "task")));
                if opening {
                    generic += 1;
                } else if closing {
                    generic -= 1;
                }
                if a.class == Class::Punct {
                    match a.text.as_str() {
                        "(" => {
                            stack.push(Open::Paren { when: pending_when });
                            pending_when = false;
                        }
                        "[" => stack.push(Open::Bracket),
                        "{" => {
                            let decl = pending_decl
                                || (was_closed_when && last.as_ref().is_some_and(|p| p.text == ")"))
                                || last.as_ref().is_some_and(|p| p.class == Class::Ident);
                            stack.push(if decl { Open::Decl } else { Open::Block });
                            pending_decl = false;
                        }
                        ")" => {
                            if let Some(Open::Paren { when }) = stack.pop() {
                                closed_when = when;
                            }
                        }
                        "]" | "}" => {
                            stack.pop();
                        }
                        _ => {}
                    }
                }
                if a.class == Class::Keyword && matches!(a.text.as_str(), "struct" | "enum") {
                    pending_decl = true;
                }
                if a.class == Class::Keyword && a.text == "when" {
                    pending_when = true;
                }
                let cont = match a.class {
                    Class::Punct => !matches!(a.text.as_str(), ")" | "]" | "}" | "++" | "--" | "?") && !closing,
                    Class::Keyword => !matches!(
                        a.text.as_str(),
                        "return" | "break" | "continue" | "true" | "false" | "int" | "float" | "bool" | "string" | "void" | "array"
                    ),
                    Class::Ident => a.text == "install",
                    _ => false,
                };
                fn_name_prev = a.class == Class::Ident && last.as_ref().is_some_and(|p| p.text == "fn");
                out.push_str(&a.text);
                last = Some(a.clone());
                last_cont = cont;
                newline = false;
            }
        }
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

fn check_compact(src: &str, keep_comments: bool) -> Result<String, FmtError> {
    let before = token_kinds(src).map_err(FmtError::Lex)?;
    let out = format_compact(src, keep_comments);
    let after = token_kinds(&out).map_err(FmtError::Lex)?;
    // the only token the compact layout adds is the `;` that replaces a line break
    let without_semis = |kinds: Vec<TokenKind>| -> Vec<TokenKind> {
        kinds.into_iter().filter(|k| *k != TokenKind::Semicolon).collect()
    };
    if without_semis(after) != without_semis(before) {
        return Err(FmtError::TokensChanged);
    }
    let want = if keep_comments { comments(src) } else { Vec::new() };
    if comments(&out) != want {
        return Err(FmtError::CommentsChanged);
    }
    match (spg_lexer::read_file_spec(src), spg_lexer::read_file_spec(&out)) {
        (Ok(a), Ok(b)) if a.friendly_terminal == b.friendly_terminal && a.strict == b.strict && a.timeout_ms == b.timeout_ms => {}
        _ => return Err(FmtError::DirectivesChanged),
    }
    // the same program, read again: the tree must match once the positions are blanked
    if let Some(shape) = program_shape(src) {
        if program_shape(&out) != Some(shape) {
            return Err(FmtError::AstChanged);
        }
    }
    if format_compact(&out, keep_comments) != out {
        return Err(FmtError::NotStable);
    }
    Ok(out)
}

/// the parsed program with its positions blanked: two texts that parse to the same
/// tree have the same shape, however their lines and columns differ
fn program_shape(src: &str) -> Option<String> {
    let tokens = spg_lexer::Lexer::new(src).tokenize().ok()?;
    let program = spg_parser::Parser::new(tokens).parse_program().ok()?;
    Some(strip_positions(&format!("{:?}", program)))
}

/// blanks the positions. the parser also names its temporaries after where they
/// are (`$let_7_1`), so those digits go too: the names only need to be unique
fn strip_positions(debug: &str) -> String {
    let mut out = String::with_capacity(debug.len());
    let mut rest = debug;
    while let Some(at) = rest.find("Pos {") {
        out.push_str(&rest[..at]);
        out.push_str("Pos");
        match rest[at..].find('}') {
            Some(end) => rest = &rest[at + end + 1..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    blank_temp_positions(&out)
}

/// `$name_12_34` becomes `$name`: a generated name with its line and column cut off
fn blank_temp_positions(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        out.push(c);
        if c != '$' {
            continue;
        }
        while let Some(&n) = chars.peek() {
            if n.is_ascii_alphabetic() {
                out.push(n);
                chars.next();
            } else {
                break;
            }
        }
        // the `_line_col` tail: `_` then digits, repeated
        loop {
            let mut ahead = chars.clone();
            if ahead.next() != Some('_') {
                break;
            }
            let digits = ahead.clone().take_while(|d| d.is_ascii_digit()).count();
            if digits == 0 {
                break;
            }
            for _ in 0..=digits {
                chars.next();
            }
        }
    }
    out
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
    fn function_type_parameters_hug_the_name() {
        assert_eq!(fmt("fn add<T: Numeric>(n: T) -> T {\n    return n\n}\n"), "fn add<T: Numeric>(n: T) -> T {\n    return n\n}\n");
        assert_eq!(fmt("fn f<A: int or float, B>(a: A, b: B) -> A {\n    return a\n}\n"), "fn f<A: int || float, B>(a: A, b: B) -> A {\n    return a\n}\n");
    }

    #[test]
    fn or_is_canonicalized_in_unions_and_operators() {
        assert_eq!(fmt("fn show(v: int or float) {\n    print(v)\n}\n"), "fn show(v: int || float) {\n    print(v)\n}\n");
        assert_eq!(fmt("let a = x or y\n"), "let a = x || y\n");
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

#[cfg(test)]
mod compact_tests {
    use super::*;

    fn min(src: &str) -> String {
        format_compact(src, true)
    }

    fn min_nc(src: &str) -> String {
        format_compact(src, false)
    }

    #[test]
    fn spaces_go_where_they_are_not_needed() {
        assert_eq!(min("let arr = [1, 2, 3]\n"), "let arr=[1,2,3]\n");
        assert_eq!(min("print (\"something\" )\n"), "print(\"something\")\n");
        assert_eq!(min("let n = -3\n"), "let n=-3\n");
        assert_eq!(min("let a = x - -3\n"), "let a=x- -3\n");
    }

    #[test]
    fn spaces_stay_between_words_and_fusing_operators() {
        assert_eq!(min("let x = 2 if (x) {}\n"), "let x=2 if(x){}\n");
        assert_eq!(min("let a = b = =c\n"), "let a=b= =c\n");
        assert_eq!(min("return x\n"), "return x\n");
    }

    #[test]
    fn the_example_of_the_block_collapses_to_one_line() {
        let src = "let ex = 2\nif (ex == 2) {\n   print(true)\n} else {\n   print(false)\n}\n";
        assert_eq!(min(src), "let ex=2;if(ex==2){print(true)}else{print(false)}\n");
    }

    #[test]
    fn comments_stay_on_their_own_line_with_min() {
        let src = "// this prints something\n\nprint (\"something\" )\n";
        assert_eq!(min(src), "// this prints something\nprint(\"something\")\n");
        assert_eq!(min_nc(src), "print(\"something\")\n");
    }

    #[test]
    fn a_trailing_comment_keeps_its_line() {
        assert_eq!(min("let a = 1 // lado\nlet b = 2\n"), "let a=1 // lado\nlet b=2\n");
    }

    #[test]
    fn a_line_break_before_a_paren_or_minus_stays_a_statement_break() {
        assert_eq!(min("f\n(0)\n"), "f;(0)\n");
        assert_eq!(min("let a = b\n-3\n"), "let a=b;-3\n");
    }

    #[test]
    fn an_operator_at_the_start_of_a_line_still_continues_the_expression() {
        assert_eq!(min("let x = a\n+ b\n"), "let x=a+b\n");
        assert_eq!(min("let x =\n 5\n"), "let x=5\n");
    }

    #[test]
    fn else_and_catch_stay_attached_to_their_block() {
        assert_eq!(min("try {\nthrow \"x\"\n}\ncatch (e) {\n}\n"), "try{throw \"x\"}catch(e){}\n");
    }

    #[test]
    fn return_ends_at_the_line_break() {
        assert_eq!(min("fn f() -> int {\n    return\n}\n"), "fn f()->int{return}\n");
        assert_eq!(min("fn f() -> int {\n    return\n    1\n}\n"), "fn f()->int{return;1}\n");
    }

    #[test]
    fn breaks_inside_parens_and_brackets_disappear() {
        assert_eq!(min("let xs = [\n 1,\n 2\n]\n"), "let xs=[1,2]\n");
        assert_eq!(min("print(\n 1,\n 2\n)\n"), "print(1,2)\n");
    }

    #[test]
    fn strings_come_out_byte_for_byte() {
        let src = "print(\"a  ${ x + 1 }\")\n";
        assert_eq!(min(src), "print(\"a  ${ x + 1 }\")\n");
    }

    #[test]
    fn directives_keep_their_own_line() {
        let src = "#> safe-party\nlet a = 1\n";
        assert_eq!(min(src), "#> safe-party\nlet a=1\n");
    }

    #[test]
    fn check_compact_accepts_the_examples_and_rejects_nothing_it_should_not() {
        let src = "let ex = 2\nif (ex == 2) {\n   print(true)\n} else {\n   print(false)\n}\n";
        assert!(check_layout(src, Layout::Compact { keep_comments: true }).is_ok());
        assert!(check_layout(src, Layout::Compact { keep_comments: false }).is_ok());
    }
}

#[cfg(test)]
mod block_comment_tests {
    use super::*;

    #[test]
    fn block_comments_follow_the_same_rule_as_line_comments() {
        let src = "/{ secret\n  two lines }/\nlet a = 1\n";
        assert_eq!(format_compact(src, false), "let a=1\n");
        assert_eq!(format_compact(src, true), "/{ secret\n  two lines }/ let a=1\n");
        assert!(check_layout(src, Layout::Compact { keep_comments: false }).is_ok());
        assert!(check_layout(src, Layout::Compact { keep_comments: true }).is_ok());
    }

    #[test]
    fn canonical_keeps_block_comments() {
        assert!(format_source("/{ note }/\nlet a = 1\n").contains("/{ note }/"));
    }
}
