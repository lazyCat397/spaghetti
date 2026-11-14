//! turns .spg source into a stream of tokens. the first step of the pain.

use crate::token::{lookup_keyword, Token, TokenKind};
use std::fmt;

#[derive(Debug)]
pub enum LexError {
    UnexpectedChar { ch: char, line: usize, col: usize },
    /// a number literal that cannot be read: `0x`, `0b102`, `0xFFFFFFFFFFFFFFFFFF`, `1e400`
    BadNumber { text: String, reason: &'static str, line: usize, col: usize },
    UnterminatedString { line: usize, col: usize },
    UnterminatedBlockComment { line: usize, col: usize },
    /// `"${}"`: an interpolation with nothing inside the braces. empty, like my coffee cup.
    EmptyInterpolation { line: usize, col: usize },
    /// `@foo` where `foo` isn't a builtin namespace (or a lone `@`). typo, probably.
    UnknownNamespace { name: String, line: usize, col: usize },
    /// `#> name` where `name` isn't a known directive. `suggestion` is the
    /// closest known one, when it looks like a plausible typo
    UnknownDirective { name: String, suggestion: Option<&'static str>, line: usize, col: usize },
    /// the same directive written twice in one file. copy paste detected.
    DuplicateDirective { name: String, line: usize, col: usize },
    /// `#>` after the header (directives only live at the top of a file)
    MisplacedDirective { line: usize, col: usize },
    /// a directive with a missing, wrong or unexpected arg
    BadDirectiveArgument { name: String, expected: &'static str, line: usize, col: usize },
}

/// every `#>` directive the engine understands. the whole list. three of them.
/// the most nested calls `#> stack N` may ask for. the same as spg-vm's MAX_CALL_DEPTH (the native
/// stack of the run is sized for it), and spg-vm checks the two agree
pub const MAX_STACK_LIMIT: u64 = 100_000;

pub const DIRECTIVES: [&str; 7] = ["friendly-terminal", "timeout", "strict", "safe-party", "no-jit", "stack", "seed"];

/// what a file's `#>` header asked for. the file's wishlist.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct FileSpec {
    pub friendly_terminal: bool,
    /// `#> strict`: ambiguities become errors (int overflow, hidden names, dead code)
    pub strict: bool,
    /// `#> safe-party`: `ps.run` and `cmd.run` are off, and `ps.runFile` only takes a .ps1 inside the script's folder
    pub safe_party: bool,
    /// `#> timeout N`: the program gets shot after N milliseconds
    pub timeout_ms: Option<u64>,
    /// `#> no-jit`: this file runs in the VM only, without the native code
    pub no_jit: bool,
    /// `#> stack N`: how many nested calls the program may make (1 to 100000)
    pub stack_limit: Option<usize>,
    /// `#> seed N`: the random numbers start from this seed, so the runs repeat
    pub seed: Option<u64>,
    /// every directive written, in order: (name, line, column)
    pub directives: Vec<(String, usize, usize)>,
}

impl fmt::Display for LexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LexError::UnexpectedChar { ch, line, col } => {
                write!(f, "unexpected character '{}' at line {}, column {}", ch, line, col)
            }
            LexError::BadNumber { text, reason, line, col } => {
                write!(f, "invalid number '{}' at line {}, column {}: {}", text, line, col, reason)
            }
            LexError::EmptyInterpolation { line, col } => {
                write!(f, "empty interpolation in string, started at line {}, column {}", line, col)
            }
            LexError::UnterminatedString { line, col } => {
                write!(f, "unterminated string, started at line {}, column {}", line, col)
            }
            LexError::UnterminatedBlockComment { line, col } => {
                write!(f, "unterminated block comment, started at line {}, column {} (close it with }}/)", line, col)
            }
            LexError::UnknownNamespace { name, line, col } if crate::is_install_namespace(name) => write!(
                f,
                "'@{0}' is not built in (line {1}, column {2}); add `attach #{0}` and write it without the '@': {0}.member(...)",
                name, line, col
            ),
            LexError::UnknownNamespace { name, line, col } => {
                let list: Vec<String> = crate::namespaces().iter().map(|n| format!("@{}", n)).collect();
                if name.is_empty() {
                    write!(
                        f,
                        "'@' must be followed by a namespace name ({}) at line {}, column {}",
                        list.join(", "), line, col
                    )
                } else {
                    write!(
                        f,
                        "unknown namespace '@{}' at line {}, column {} (available: {})",
                        name, line, col, list.join(", ")
                    )
                }
            }
            LexError::UnknownDirective { name, suggestion, line, col } => {
                if name.is_empty() {
                    write!(f, "'#>' must be followed by a directive name (available: {}) at line {}, column {}", DIRECTIVES.join(", "), line, col)
                } else {
                    write!(f, "unknown directive '#> {}' at line {}, column {}", name, line, col)?;
                    match suggestion {
                        Some(s) => write!(f, ", did you mean '#> {}'?", s),
                        None => Ok(()),
                    }
                }
            }
            LexError::DuplicateDirective { name, line, col } => {
                write!(f, "directive '#> {}' is written twice (duplicate at line {}, column {})", name, line, col)
            }
            LexError::MisplacedDirective { line, col } => {
                write!(f, "'#>' directive after code at line {}, column {}; directives must come before any code", line, col)
            }
            LexError::BadDirectiveArgument { name, expected, line, col } => {
                write!(f, "directive '#> {}' expects {} (line {}, column {})", name, expected, line, col)
            }
        }
    }
}

impl LexError {
    /// the same fact as `Display`, in the friendly tone of `#> friendly-terminal`:
    /// softer wording, and always a hint about what to check
    pub fn friendly(&self, file: &str) -> String {
        match self {
            LexError::UnexpectedChar { ch, line, col } => format!(
                "Oops! I found a character I don't expect: '{}' in `{}` at line {}, column {}. Check for a typo or a stray symbol.",
                ch, file, line, col
            ),
            LexError::BadNumber { text, reason, line, col } => format!(
                "Oops! The number `{}` in `{}` at line {}, column {} is not valid: {}.",
                text, file, line, col, reason
            ),
            LexError::EmptyInterpolation { line, col } => format!(
                "Oops! Empty `${{}}` in string in `{}` at line {}, column {}. Put an expression between the braces, or write `\\${{` for a literal dollar sign.",
                file, line, col
            ),
            LexError::UnterminatedBlockComment { line, col } => format!(
                "Oops! The block comment that starts at line {}, column {} in `{}` is never closed. Add `}}/` where it should end.",
                line, col, file
            ),
            LexError::UnterminatedString { line, col } => format!(
                "Oops! Unterminated string in `{}` at line {}, column {}. Check for a missing closing quote.",
                file, line, col
            ),
            LexError::UnknownNamespace { name, line, col } if crate::is_install_namespace(name) => format!(
                "Oops! `@{0}` is not built in, in `{1}` at line {2}, column {3}. Add `attach #{0}` at the top and write it without the '@': `{0}.member(...)`.",
                name, file, line, col
            ),
            LexError::UnknownNamespace { name, line, col } => {
                let list: Vec<String> = crate::namespaces().iter().map(|n| format!("@{}", n)).collect();
                if name.is_empty() {
                    format!("Oops! A '@' needs a namespace name after it in `{}` at line {}, column {}. Available: {}.", file, line, col, list.join(", "))
                } else {
                    format!("Oops! I don't know the namespace '@{}' in `{}` at line {}, column {}. Available: {}.", name, file, line, col, list.join(", "))
                }
            }
            LexError::UnknownDirective { name, suggestion, line, col } => {
                if name.is_empty() {
                    format!("Oops! '#>' needs a directive name after it in `{}` at line {}, column {}. Available: {}.", file, line, col, DIRECTIVES.join(", "))
                } else {
                    let hint = match suggestion {
                        Some(s) => format!("Did you mean '#> {}'?", s),
                        None => format!("Available directives: {}.", DIRECTIVES.join(", ")),
                    };
                    format!("Oops! I don't know the directive '#> {}' in `{}` at line {}, column {}. {}", name, file, line, col, hint)
                }
            }
            LexError::DuplicateDirective { name, line, col } => format!(
                "Oops! The directive '#> {}' is written twice in `{}` (second time at line {}, column {}). Keep just one.",
                name, file, line, col
            ),
            LexError::MisplacedDirective { line, col } => format!(
                "Oops! '#>' directives must go at the very top of the file, before any code. Found one in `{}` at line {}, column {}. Move it up to the first lines.",
                file, line, col
            ),
            LexError::BadDirectiveArgument { name, expected, line, col } => format!(
                "Oops! The directive '#> {}' in `{}` at line {}, column {} expects {}. Check how it is written.",
                name, file, line, col, expected
            ),
        }
    }
}

/// the closest directive name to a typo, if it's at most two edits away
fn closest_directive(name: &str) -> Option<&'static str> {
    DIRECTIVES
        .iter()
        .copied()
        .map(|d| (edit_distance(name, d), d))
        .filter(|(dist, _)| *dist <= 2)
        .min()
        .map(|(_, d)| d)
}

fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let cost = if ca == *cb { 0 } else { 1 };
            cur.push((prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

impl std::error::Error for LexError {}

/// one piece of an interpolated string, while we're reading it
enum Part {
    Text(String),
    Expr(Vec<Token>),
}

/// `attach #ps` makes `ps.run(...)` a namespace call (no `@`). this pass finds the
/// installs of native libraries in a token stream and turns each `ps` `.` (not itself a
/// member, like `x.ps.y`) into a `Namespace` token, so the parser, checker and VM keep seeing
/// the usual qualified name. interpolated expressions are already spliced into the stream.
/// public so the editor tooling (which lexes on its own) can reuse it.
pub fn apply_installed_namespaces(tokens: &mut [Token]) {
    let mut installed: Vec<String> = Vec::new();
    for w in tokens.windows(2) {
        if let [Token { kind: TokenKind::Attach, .. }, Token { kind: TokenKind::StdLib(name), .. }] = w {
            if crate::is_install_namespace(name) && !installed.contains(name) {
                installed.push(name.clone());
            }
        }
    }
    if installed.is_empty() {
        return;
    }
    for i in 0..tokens.len().saturating_sub(1) {
        let TokenKind::Ident(name) = &tokens[i].kind else { continue };
        if !installed.contains(name) || tokens[i + 1].kind != TokenKind::Dot {
            continue;
        }
        if i > 0 && tokens[i - 1].kind == TokenKind::Dot {
            continue;
        }
        let name = name.clone();
        tokens[i].kind = TokenKind::Namespace(name);
    }
}

/// tokenizes a piece of source at a known position (an interpolated
/// expression, or a fake wrapper), so its tokens report real positions
fn fragment_tokens(src: &str, line: usize, col: usize) -> Result<Vec<Token>, LexError> {
    let mut lx = Lexer::new(src);
    lx.line = line;
    lx.col = col;
    lx.lex_tokens()
}

pub struct Lexer<'a> {
    /// decoded characters, NOT bytes: a multi-byte UTF-8 char (`á`, `ç`, `—`)
    /// is one element, one column, and survives into string literals intact. accents, respect.
    source: Vec<char>,
    _src: std::marker::PhantomData<&'a str>,
    pos: usize,
    line: usize,
    col: usize,
    spec: FileSpec,
}

impl<'a> Lexer<'a> {
    pub fn new(source: &'a str) -> Self {
        Lexer {
            source: source.chars().collect(),
            _src: std::marker::PhantomData,
            pos: 0,
            line: 1,
            col: 1,
            spec: FileSpec::default(),
        }
    }

    pub fn tokenize(self) -> Result<Vec<Token>, LexError> {
        self.tokenize_with_spec().map(|(tokens, _)| tokens)
    }

    /// tokens plus the file's `#>` header. directives get read first, and are
    /// not tokens. the parser never sees them. lucky parser.
    pub fn tokenize_with_spec(mut self) -> Result<(Vec<Token>, FileSpec), LexError> {
        self.read_header()?;
        let mut tokens = self.lex_tokens()?;
        apply_installed_namespaces(&mut tokens);
        let (line, col) = (self.line, self.col);
        tokens.push(Token::new(TokenKind::Eof, line, col));
        Ok((tokens, self.spec))
    }

    /// every token up to the end of the input, WITHOUT the Eof marker
    fn lex_tokens(&mut self) -> Result<Vec<Token>, LexError> {
        let mut tokens = Vec::new();

        loop {
            self.skip_whitespace_and_comments()?;

            let (line, col) = (self.line, self.col);

            let Some(ch) = self.peek() else {
                break;
            };

            // the header was already read, so a directive here is misplaced
            if ch == '#' && self.peek_next() == Some('>') {
                return Err(LexError::MisplacedDirective { line, col });
            }

            // `#name`: a standard library, as in `attach #http`
            if ch == '#' && self.peek_next().is_some_and(|c| c.is_alphabetic() || c == '_') {
                self.advance(); // #
                let mut name = String::new();
                while let Some(c) = self.peek() {
                    if c.is_alphanumeric() || c == '_' {
                        name.push(c);
                        self.advance();
                    } else {
                        break;
                    }
                }
                tokens.push(Token::new(TokenKind::StdLib(name), line, col));
                continue;
            }

            if ch == '"' {
                let parts = self.read_string_tokens()?;
                tokens.extend(parts);
                continue;
            }

            let kind = if ch.is_ascii_digit() {
                self.read_number()?
            } else if ch == '\'' {
                self.read_raw_string()?
            } else if ch.is_alphabetic() || ch == '_' {
                self.read_ident_or_keyword()
            } else if ch == '@' {
                self.read_namespace()?
            } else {
                self.read_operator_or_delim(ch)?
            };

            tokens.push(Token::new(kind, line, col));
        }

        Ok(tokens)
    }

    /// reads the `#>` header: directive lines before any code (comments and
    /// blank lines may sit between them. air is allowed.
    fn read_header(&mut self) -> Result<(), LexError> {
        loop {
            self.skip_whitespace_and_comments()?;
            if self.peek() == Some('#') && self.peek_next() == Some('>') {
                self.read_directive()?;
            } else {
                return Ok(());
            }
        }
    }

    fn read_directive(&mut self) -> Result<(), LexError> {
        let (line, col) = (self.line, self.col);
        self.advance();
        self.advance();
        while matches!(self.peek(), Some(' ' | '\t')) {
            self.advance();
        }
        let mut name = String::new();
        while let Some(c) = self.peek() {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                name.push(c);
                self.advance();
            } else {
                break;
            }
        }
        if name.is_empty() {
            return Err(LexError::UnknownDirective { name, suggestion: None, line, col });
        }
        if !DIRECTIVES.contains(&name.as_str()) {
            let suggestion = closest_directive(&name);
            return Err(LexError::UnknownDirective { name, suggestion, line, col });
        }
        // `timeout` takes a whole number of ms. the others take nothing. simple.
        while matches!(self.peek(), Some(' ' | '\t')) {
            self.advance();
        }
        let mut number: Option<u64> = None;
        if matches!(name.as_str(), "timeout" | "stack" | "seed") {
            let mut digits = String::new();
            while let Some(c) = self.peek() {
                if c.is_ascii_digit() {
                    digits.push(c);
                    self.advance();
                } else {
                    break;
                }
            }
            let parsed = digits.parse::<u64>().ok().filter(|n| match name.as_str() {
                "timeout" => *n > 0,
                "stack" => (1..=MAX_STACK_LIMIT).contains(n),
                _ => true,
            });
            match parsed {
                Some(n) => number = Some(n),
                None => {
                    let expected = match name.as_str() {
                        "timeout" => "a whole number of milliseconds greater than 0, like `#> timeout 5000`",
                        "stack" => "a whole number of calls from 1 to 100000, like `#> stack 5000`",
                        _ => "a whole number, like `#> seed 42`",
                    };
                    return Err(LexError::BadDirectiveArgument { name, expected, line, col });
                }
            }
        }
        // a directive owns its whole line. only spaces or a comment may follow
        while matches!(self.peek(), Some(' ' | '\t' | '\r')) {
            self.advance();
        }
        match self.peek() {
            None | Some('\n') => {}
            Some('/') if self.peek_next() == Some('/') => {}
            Some(_) => {
                let expected = match name.as_str() {
                    "timeout" => "just the number of milliseconds, nothing else on the line",
                    "stack" | "seed" => "just the number, nothing else on the line",
                    _ => "no argument",
                };
                return Err(LexError::BadDirectiveArgument { name, expected, line, col });
            }
        }
        if self.spec.directives.iter().any(|(n, _, _)| *n == name) {
            return Err(LexError::DuplicateDirective { name, line, col });
        }
        match name.as_str() {
            "friendly-terminal" => self.spec.friendly_terminal = true,
            "strict" => self.spec.strict = true,
            "safe-party" => self.spec.safe_party = true,
            "no-jit" => self.spec.no_jit = true,
            "stack" => self.spec.stack_limit = number.map(|n| n as usize),
            "seed" => self.spec.seed = number,
            _ => self.spec.timeout_ms = number,
        }
        self.spec.directives.push((name, line, col));
        Ok(())
    }

    fn peek(&self) -> Option<char> {
        self.source.get(self.pos).copied()
    }

    fn peek_next(&self) -> Option<char> {
        self.source.get(self.pos + 1).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.pos += 1;
        if ch == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(ch)
    }

    /// true when the source at `pos + offset` continues with exactly `text`
    fn at(&self, offset: usize, text: &str) -> bool {
        text.chars()
            .enumerate()
            .all(|(i, c)| self.source.get(self.pos + offset + i) == Some(&c))
    }

    fn skip_whitespace_and_comments(&mut self) -> Result<(), LexError> {
        loop {
            match self.peek() {
                Some(c) if c.is_whitespace() => {
                    self.advance();
                }
                // block comment: /{ ... }/  (no nesting. the first }/ closes it, no questions)
                Some('/') if self.at(0, "/{") => {
                    let (line, col) = (self.line, self.col);
                    for _ in 0..2 {
                        self.advance();
                    }
                    loop {
                        if self.peek().is_none() {
                            return Err(LexError::UnterminatedBlockComment { line, col });
                        }
                        if self.at(0, "}/") {
                            for _ in 0..2 {
                                self.advance();
                            }
                            break;
                        }
                        self.advance();
                    }
                }
                Some('/') if self.peek_next() == Some('/') => {
                    // line comment: // ... (the classic)
                    while let Some(c) = self.peek() {
                        if c == '\n' {
                            break;
                        }
                        self.advance();
                    }
                }
                _ => break,
            }
        }
        Ok(())
    }

    /// `_` is a digit separator only when a digit of the same kind follows it: `1_000`, `1__000`.
    /// `1_` and `1_x` stop before the underscore, as before.
    fn at_digit_separator(&self, is_digit: fn(char) -> bool) -> bool {
        let mut i = self.pos;
        while self.source.get(i) == Some(&'_') {
            i += 1;
        }
        i > self.pos && self.source.get(i).map_or(false, |c| is_digit(*c))
    }

    /// reads a run of digits, skipping the separators between them
    fn read_digit_run(&mut self, text: &mut String, is_digit: fn(char) -> bool) {
        loop {
            match self.peek() {
                Some(c) if is_digit(c) => {
                    text.push(c);
                    self.advance();
                }
                Some('_') if self.at_digit_separator(is_digit) => {
                    while self.peek() == Some('_') {
                        self.advance();
                    }
                }
                _ => break,
            }
        }
    }

    /// reads a number literal:
    /// decimal `1_000`, `3.14`, `1e6` (a float), `2.5e-3`; hex `0xFF_00`; binary `0b1010`.
    /// the value is what the digits say, separators aside.
    fn read_number(&mut self) -> Result<TokenKind, LexError> {
        let (line, col) = (self.line, self.col);
        let start = self.pos;

        // `0x` / `0b`: every letter and digit after the prefix belongs to the literal, so a
        // wrong digit gives one clear error instead of a stray name
        if self.peek() == Some('0') {
            let radix = match self.peek_next() {
                Some('x') => Some(16),
                Some('b') => Some(2),
                _ => None,
            };
            if let Some(radix) = radix {
                self.advance(); // 0
                self.advance(); // x or b
                let digits_start = self.pos;
                while let Some(c) = self.peek() {
                    if c.is_ascii_alphanumeric() || c == '_' {
                        self.advance();
                    } else {
                        break;
                    }
                }
                let text: String = self.source[start..self.pos].iter().collect();
                let digits: String = self.source[digits_start..self.pos].iter().filter(|c| **c != '_').collect();
                return match i64::from_str_radix(&digits, radix) {
                    Ok(n) => Ok(TokenKind::Int(n)),
                    Err(e) => {
                        let reason = match e.kind() {
                            std::num::IntErrorKind::Empty => "there must be digits after the prefix",
                            std::num::IntErrorKind::PosOverflow => "the value is too big for an int",
                            _ => "a character is not a digit of this base",
                        };
                        Err(LexError::BadNumber { text, reason, line, col })
                    }
                };
            }
        }

        let mut text = String::new();
        let mut is_float = false;

        self.read_digit_run(&mut text, |c| c.is_ascii_digit());

        if self.peek() == Some('.') && self.peek_next().map_or(false, |n| n.is_ascii_digit()) {
            is_float = true;
            text.push('.');
            self.advance();
            self.read_digit_run(&mut text, |c| c.is_ascii_digit());
        }

        // exponent: `e` or `E`, an optional sign, then digits. without digits the `e` stays a
        // name, so `1e` is still an error, as before
        if matches!(self.peek(), Some('e') | Some('E')) {
            let sign = matches!(self.peek_next(), Some('+') | Some('-'));
            let first_digit = self.pos + if sign { 2 } else { 1 };
            if self.source.get(first_digit).map_or(false, |c| c.is_ascii_digit()) {
                is_float = true;
                text.push('e');
                self.advance();
                if let Some(s @ ('+' | '-')) = self.peek() {
                    text.push(s);
                    self.advance();
                }
                self.read_digit_run(&mut text, |c| c.is_ascii_digit());
            }
        }

        let raw: String = self.source[start..self.pos].iter().collect();
        if is_float {
            match text.parse::<f64>() {
                Ok(v) if v.is_finite() => Ok(TokenKind::Float(v)),
                _ => Err(LexError::BadNumber { text: raw, reason: "the value is too big for a float", line, col }),
            }
        } else {
            match text.parse::<i64>() {
                Ok(n) => Ok(TokenKind::Int(n)),
                Err(_) => Err(LexError::BadNumber { text: raw, reason: "the value is too big for an int", line, col }),
            }
        }
    }

    /// `"..."`: a normal string. escapes work as before, and `${expr}`
    /// interpolates. the literal becomes a parenthesized `+` chain of string
    /// pieces and `@into.string(expr)` calls, so the parser and checker
    /// see ordinary tokens. `\${` is just text (`\$` is an escape like any other).
    fn read_string_tokens(&mut self) -> Result<Vec<Token>, LexError> {
        let (start_line, start_col) = (self.line, self.col);
        self.advance(); // consume the opening "

        let mut parts: Vec<Part> = Vec::new();
        let mut text = String::new();
        let mut has_expr = false;
        loop {
            match self.peek() {
                None => {
                    return Err(LexError::UnterminatedString {
                        line: start_line,
                        col: start_col,
                    })
                }
                Some('"') => {
                    self.advance();
                    break;
                }
                Some('\\') => {
                    self.advance();
                    match self.advance() {
                        Some('n') => text.push('\n'),
                        Some('t') => text.push('\t'),
                        Some('r') => text.push('\r'),
                        Some('"') => text.push('"'),
                        Some('\\') => text.push('\\'),
                        Some(other) => text.push(other),
                        None => {
                            return Err(LexError::UnterminatedString {
                                line: start_line,
                                col: start_col,
                            })
                        }
                    }
                }
                Some('$') if self.peek_next() == Some('{') => {
                    self.advance();
                    self.advance();
                    let (expr_line, expr_col) = (self.line, self.col);
                    let src = self.read_interpolation(start_line, start_col)?;
                    if src.trim().is_empty() {
                        return Err(LexError::EmptyInterpolation {
                            line: start_line,
                            col: start_col,
                        });
                    }
                    if !text.is_empty() {
                        parts.push(Part::Text(std::mem::take(&mut text)));
                    }
                    parts.push(Part::Expr(fragment_tokens(&src, expr_line, expr_col)?));
                    has_expr = true;
                }
                Some(c) => {
                    text.push(c);
                    self.advance();
                }
            }
        }

        if !has_expr {
            return Ok(vec![Token::new(TokenKind::Str(text), start_line, start_col)]);
        }
        if !text.is_empty() {
            parts.push(Part::Text(text));
        }

        let (l, c) = (start_line, start_col);
        let mut out = fragment_tokens("(", l, c)?;
        for (i, part) in parts.into_iter().enumerate() {
            if i > 0 {
                out.extend(fragment_tokens("+", l, c)?);
            }
            match part {
                Part::Text(t) => out.push(Token::new(TokenKind::Str(t), l, c)),
                Part::Expr(toks) => {
                    out.extend(fragment_tokens("@into.string(", l, c)?);
                    out.extend(toks);
                    out.extend(fragment_tokens(")", l, c)?);
                }
            }
        }
        out.extend(fragment_tokens(")", l, c)?);
        Ok(out)
    }

    /// raw source of `${...}`, up to its closing brace (the `${` is already
    /// consumed). nested braces, strings and raw strings get copied whole, so
    /// `"${@str.len("}")}"` closes at the right brace. yes, even that one.
    fn read_interpolation(&mut self, start_line: usize, start_col: usize) -> Result<String, LexError> {
        let unterminated = LexError::UnterminatedString {
            line: start_line,
            col: start_col,
        };
        let mut raw = String::new();
        let mut depth = 0usize;
        loop {
            let Some(c) = self.advance() else {
                return Err(unterminated);
            };
            match c {
                '}' if depth == 0 => return Ok(raw),
                '}' => {
                    depth -= 1;
                    raw.push(c);
                }
                '{' => {
                    depth += 1;
                    raw.push(c);
                }
                '"' => {
                    raw.push(c);
                    self.copy_nested_string(&mut raw, start_line, start_col)?;
                }
                '\'' => {
                    raw.push(c);
                    loop {
                        match self.advance() {
                            None => return Err(unterminated),
                            Some('\'') => {
                                raw.push('\'');
                                break;
                            }
                            Some(x) => raw.push(x),
                        }
                    }
                }
                _ => raw.push(c),
            }
        }
    }

    /// the rest of a `"..."` inside `${...}`, copied raw through its
    /// closing quote (nested `${...}` included)
    fn copy_nested_string(&mut self, raw: &mut String, start_line: usize, start_col: usize) -> Result<(), LexError> {
        let unterminated = LexError::UnterminatedString {
            line: start_line,
            col: start_col,
        };
        loop {
            let Some(c) = self.advance() else {
                return Err(unterminated);
            };
            raw.push(c);
            match c {
                '"' => return Ok(()),
                '\\' => {
                    let Some(n) = self.advance() else {
                        return Err(unterminated);
                    };
                    raw.push(n);
                }
                '$' if self.peek() == Some('{') => {
                    self.advance();
                    raw.push('{');
                    let inner = self.read_interpolation(start_line, start_col)?;
                    raw.push_str(&inner);
                    raw.push('}');
                }
                _ => {}
            }
        }
    }

    /// `'...'`: a RAW string. everything up to the next `'` is the content,
    /// byte for byte: no escapes at all (`'a\nb'` is the 4 characters
    /// `a`, `\`, `n`, `b`), and a `"` inside is just a `"`. because there is
    /// no escape, a `'` can never appear inside. the first `'` after
    /// the opening one always closes it. deliberate: the lexer stays
    /// stuck in the string it already opened, never looking ahead for a
    /// "better" closing quote, so `'print('hi')'` fails instead of silently
    /// gluing strings together. no sneaky merging.
    fn read_raw_string(&mut self) -> Result<TokenKind, LexError> {
        let (start_line, start_col) = (self.line, self.col);
        self.advance(); // consume the opening '

        let mut s = String::new();
        loop {
            match self.advance() {
                None => {
                    return Err(LexError::UnterminatedString { line: start_line, col: start_col })
                }
                Some('\'') => break,
                Some(c) => s.push(c),
            }
        }

        Ok(TokenKind::Str(s))
    }

    fn read_ident_or_keyword(&mut self) -> TokenKind {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c.is_alphanumeric() || c == '_' {
                self.advance();
            } else {
                break;
            }
        }
        let text: String = self.source[start..self.pos].iter().collect();

        lookup_keyword(&text).unwrap_or(TokenKind::Ident(text))
    }

    /// `@name` → `TokenKind::Namespace(name)`. anything that's not one of
    /// `NAMESPACES` is a lex error (so a typo like `@mth` gets caught here)
    fn read_namespace(&mut self) -> Result<TokenKind, LexError> {
        let (line, col) = (self.line, self.col);
        self.advance(); // @
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c.is_alphanumeric() || c == '_' {
                self.advance();
            } else {
                break;
            }
        }
        let name: String = self.source[start..self.pos].iter().collect();
        if crate::is_namespace(&name) {
            Ok(TokenKind::Namespace(name))
        } else {
            Err(LexError::UnknownNamespace { name, line, col })
        }
    }

    fn read_operator_or_delim(&mut self, ch: char) -> Result<TokenKind, LexError> {
        let (line, col) = (self.line, self.col);
        self.advance();

        let kind = match ch {
            '+' => {
                if self.peek() == Some('=') {
                    self.advance();
                    TokenKind::PlusEq
                } else if self.peek() == Some('+') {
                    self.advance();
                    TokenKind::PlusPlus
                } else {
                    TokenKind::Plus
                }
            }
            '-' => {
                if self.peek() == Some('>') {
                    self.advance();
                    TokenKind::Arrow
                } else if self.peek() == Some('=') {
                    self.advance();
                    TokenKind::MinusEq
                } else if self.peek() == Some('-') {
                    self.advance();
                    TokenKind::MinusMinus
                } else {
                    TokenKind::Minus
                }
            }
            '*' => {
                if self.peek() == Some('=') {
                    self.advance();
                    TokenKind::StarEq
                } else if self.peek() == Some('*') {
                    self.advance();
                    TokenKind::StarStar
                } else {
                    TokenKind::Star
                }
            }
            '/' => {
                if self.peek() == Some('=') {
                    self.advance();
                    TokenKind::SlashEq
                } else {
                    TokenKind::Slash
                }
            }
            '%' => {
                if self.peek() == Some('=') {
                    self.advance();
                    TokenKind::PercentEq
                } else {
                    TokenKind::Percent
                }
            }
            '=' => {
                if self.peek() == Some('=') {
                    self.advance();
                    TokenKind::EqEq
                } else {
                    TokenKind::Eq
                }
            }
            '!' => {
                if self.peek() == Some('=') {
                    self.advance();
                    TokenKind::NotEq
                } else {
                    TokenKind::Not
                }
            }
            '<' => {
                if self.peek() == Some('=') {
                    self.advance();
                    TokenKind::LtEq
                } else {
                    TokenKind::Lt
                }
            }
            '>' => {
                if self.peek() == Some('=') {
                    self.advance();
                    TokenKind::GtEq
                } else {
                    TokenKind::Gt
                }
            }
            '?' => {
                if self.peek() == Some('?') {
                    self.advance();
                    TokenKind::QuestionQuestion
                } else {
                    TokenKind::Question
                }
            }
            '&' if self.peek() == Some('&') => {
                self.advance();
                TokenKind::AndAnd
            }
            '|' if self.peek() == Some('|') => {
                self.advance();
                TokenKind::OrOr
            }
            '|' if self.peek() == Some('>') => {
                self.advance();
                TokenKind::PipeArrow
            }
            ':' => {
                if self.peek() == Some(':') {
                    self.advance();
                    TokenKind::DoubleColon
                } else {
                    TokenKind::Colon
                }
            }
            ',' => TokenKind::Comma,
            '.' => {
                if self.peek() == Some('.') {
                    self.advance();
                    TokenKind::DotDot
                } else {
                    TokenKind::Dot
                }
            }
            '(' => TokenKind::LParen,
            ')' => TokenKind::RParen,
            '{' => TokenKind::LBrace,
            '}' => TokenKind::RBrace,
            '[' => TokenKind::LBracket,
            ']' => TokenKind::RBracket,
            ';' => TokenKind::Semicolon,
            other => return Err(LexError::UnexpectedChar { ch: other, line, col }),
        };

        Ok(kind)
    }
}

#[cfg(test)]
mod namespace_tests {
    use super::*;

    #[test]
    fn at_namespace_is_one_token() {
        let toks = Lexer::new("@math.sqrt").tokenize().unwrap();
        assert_eq!(toks[0].kind, TokenKind::Namespace("math".into()));
        assert_eq!(toks[1].kind, TokenKind::Dot);
    }

    #[test]
    fn unknown_or_bare_at_is_an_error() {
        assert!(matches!(Lexer::new("@mth.x").tokenize(), Err(LexError::UnknownNamespace { .. })));
        assert!(matches!(Lexer::new("@ ").tokenize(), Err(LexError::UnknownNamespace { .. })));
        // the old spellings are plain identifiers now. they're just words.
        let toks = Lexer::new("math").tokenize().unwrap();
        assert_eq!(toks[0].kind, TokenKind::Ident("math".into()));
    }
}

#[cfg(test)]
mod utf8_tests {
    use super::*;
    use crate::token::TokenKind;

    #[test]
    fn string_literal_keeps_multibyte_chars() {
        let toks = Lexer::new("\"olá — ção\"").tokenize().unwrap();
        assert_eq!(toks[0].kind, TokenKind::Str("olá — ção".to_string()));
    }

    #[test]
    fn columns_count_chars_not_bytes() {
        // `é` is 2 bytes. the `x` after it must still be column 8. counting matters.
        let toks = Lexer::new("\"é\" + x").tokenize().unwrap();
        let x = toks.iter().find(|t| t.kind == TokenKind::Ident("x".into())).unwrap();
        assert_eq!((x.line, x.col), (1, 7));
    }
}

#[cfg(test)]
mod raw_string_and_range_tests {
    use super::*;

    fn kinds(src: &str) -> Vec<TokenKind> {
        Lexer::new(src).tokenize().unwrap().into_iter().map(|t| t.kind).collect()
    }

    #[test]
    fn single_quotes_are_a_raw_string_that_may_contain_double_quotes() {
        assert_eq!(kinds("'print(\"hi\")'")[0], TokenKind::Str("print(\"hi\")".into()));
        // plain content works like "..." does
        assert_eq!(kinds("'hi'")[0], TokenKind::Str("hi".into()));
    }

    #[test]
    fn carriage_return_escape() {
        assert_eq!(kinds("\"a\\r\\nb\"")[0], TokenKind::Str("a\r\nb".into()));
    }

    #[test]
    fn raw_string_has_no_escapes() {
        // backslash-n stays as two characters. backslash doesn't hide the closing quote
        assert_eq!(kinds("'a\\nb'")[0], TokenKind::Str("a\\nb".into()));
        assert_eq!(kinds("'a\\'")[0], TokenKind::Str("a\\".into()));
    }

    #[test]
    fn raw_string_closes_at_the_first_quote_and_never_looks_ahead() {
        // `'print('` is already a complete string. `hi` is a plain identifier.
        // `')'` is another string. the lexer never skips ahead to a "better"
        // closing quote. the parser is the one that rejects this sequence.
        assert_eq!(
            kinds("'print('hi')'"),
            vec![
                TokenKind::Str("print(".into()),
                TokenKind::Ident("hi".into()),
                TokenKind::Str(")".into()),
                TokenKind::Eof
            ]
        );
        // with an odd number of quotes the last one is left open. good luck, user.
        assert!(matches!(
            Lexer::new("'print('hi')").tokenize(),
            Err(LexError::UnterminatedString { line: 1, col: 11 })
        ));
    }

    #[test]
    fn unterminated_raw_string_points_at_its_opening_quote() {
        assert!(matches!(
            Lexer::new("let x = 1\nlet s = 'oops").tokenize(),
            Err(LexError::UnterminatedString { line: 2, col: 9 })
        ));
    }

    #[test]
    fn raw_string_keeps_multibyte_chars_and_newlines() {
        assert_eq!(kinds("'olá\nmundo'")[0], TokenKind::Str("olá\nmundo".into()));
    }

    #[test]
    fn dotdot_is_one_token_and_does_not_eat_the_int_before_it() {
        assert_eq!(
            kinds("1..4"),
            vec![TokenKind::Int(1), TokenKind::DotDot, TokenKind::Int(4), TokenKind::Eof]
        );
        // a float still lexes as a float
        assert_eq!(kinds("1.5")[0], TokenKind::Float(1.5));
        // a field access is still a single dot
        assert_eq!(kinds("a.b")[1], TokenKind::Dot);
    }
}

/// the directive header of a file, read on its own. the CLI uses it to decide
/// how to show errors, even when the rest of the file is broken
pub fn read_file_spec(source: &str) -> Result<FileSpec, LexError> {
    let mut lexer = Lexer::new(source);
    lexer.read_header()?;
    Ok(lexer.spec)
}

#[cfg(test)]
mod directive_tests {
    use super::*;

    fn tokens_of(src: &str) -> Vec<TokenKind> {
        Lexer::new(src).tokenize().unwrap().into_iter().map(|t| t.kind).collect()
    }

    #[test]
    fn header_turns_friendly_terminal_on_with_or_without_space() {
        assert!(read_file_spec("#> friendly-terminal\nprint(1)").unwrap().friendly_terminal);
        assert!(read_file_spec("#>friendly-terminal\n").unwrap().friendly_terminal);
        assert!(!read_file_spec("print(1)").unwrap().friendly_terminal);
    }

    #[test]
    fn directives_are_not_tokens_and_comments_may_precede_them() {
        let src = "// hi\n\n#> friendly-terminal // soft\nprint(1)";
        let kinds = tokens_of(src);
        assert_eq!(kinds.len(), tokens_of("print(1)").len());
    }

    #[test]
    fn typo_gets_a_suggestion_and_unknown_name_does_not() {
        match read_file_spec("#> friendly-terminl") {
            Err(LexError::UnknownDirective { suggestion, .. }) => assert_eq!(suggestion, Some("friendly-terminal")),
            other => panic!("expected UnknownDirective, got {:?}", other),
        }
        match read_file_spec("#> sandbox") {
            Err(LexError::UnknownDirective { suggestion, .. }) => assert_eq!(suggestion, None),
            other => panic!("expected UnknownDirective, got {:?}", other),
        }
    }

    #[test]
    fn duplicate_and_misplaced_directives_are_errors() {
        assert!(matches!(
            read_file_spec("#> friendly-terminal\n#> friendly-terminal"),
            Err(LexError::DuplicateDirective { .. })
        ));
        assert!(matches!(
            Lexer::new("print(1)\n#> friendly-terminal").tokenize(),
            Err(LexError::MisplacedDirective { line: 2, col: 1 })
        ));
        assert!(matches!(
            Lexer::new("print(1) #>friendly-terminal").tokenize(),
            Err(LexError::MisplacedDirective { .. })
        ));
    }

    #[test]
    fn friendly_unterminated_string_matches_the_expected_wording() {
        let err = Lexer::new("print(\"Hello, World!)\n").tokenize().unwrap_err();
        assert_eq!(
            err.friendly("main.spg"),
            "Oops! Unterminated string in `main.spg` at line 1, column 7. Check for a missing closing quote."
        );
    }
}

#[cfg(test)]
mod directive_args_tests {
    use super::*;

    #[test]
    fn strict_and_timeout_are_read_from_the_header() {
        let spec = read_file_spec("#> strict\n#>timeout 250 // quarter second\nprint(1)").unwrap();
        assert!(spec.strict);
        assert_eq!(spec.timeout_ms, Some(250));
        assert!(!spec.friendly_terminal);
    }

    #[test]
    fn timeout_needs_a_positive_whole_number_and_strict_takes_none() {
        for bad in ["#> timeout", "#> timeout 0", "#> timeout 5s", "#> timeout abc", "#> strict 1", "#> timeout 5 6"] {
            assert!(
                matches!(read_file_spec(bad), Err(LexError::BadDirectiveArgument { .. })),
                "{:?} should be a bad argument",
                bad
            );
        }
    }

    #[test]
    fn typos_of_the_new_names_get_suggestions() {
        for (typo, want) in [("#> strikt", "strict"), ("#> timout 5", "timeout")] {
            match read_file_spec(typo) {
                Err(LexError::UnknownDirective { suggestion, .. }) => assert_eq!(suggestion, Some(want)),
                other => panic!("{:?}: expected UnknownDirective, got {:?}", typo, other),
            }
        }
    }

    #[test]
    fn block_comments_hide_code_across_lines() {
        let src = "print(1)\n/{ print(2)\n  // inner line comment, no nesting /{\n}/\nprint(3) /{ tail }/ print(4)";
        let toks: Vec<TokenKind> = Lexer::new(src).tokenize().unwrap().into_iter().map(|t| t.kind).collect();
        let text = format!("{:?}", toks);
        assert!(text.contains("Int(1)") && text.contains("Int(3)") && text.contains("Int(4)"));
        assert!(!text.contains("Int(2)"), "{}", text);
    }

    #[test]
    fn unterminated_block_comment_is_an_error_with_its_start() {
        match Lexer::new("print(1)\n  /{ never closed\nprint(2)").tokenize() {
            Err(LexError::UnterminatedBlockComment { line, col }) => assert_eq!((line, col), (2, 3)),
            other => panic!("expected UnterminatedBlockComment, got {:?}", other),
        }
    }

    #[test]
    fn block_marker_inside_a_line_comment_opens_nothing() {
        let toks: Vec<TokenKind> = Lexer::new("print(1) // note /{ is not a block\nprint(2)").tokenize().unwrap().into_iter().map(|t| t.kind).collect();
        let text = format!("{:?}", toks);
        assert!(text.contains("Int(2)"), "{}", text);
    }
}

#[cfg(test)]
mod header_value_tests {
    use crate::read_file_spec;

    #[test]
    fn the_new_directives_read_their_values() {
        let spec = read_file_spec("#> no-jit\n#> stack 50\n#> seed 7\nprint(1)\n").unwrap();
        assert!(spec.no_jit);
        assert_eq!(spec.stack_limit, Some(50));
        assert_eq!(spec.seed, Some(7));
    }

    #[test]
    fn a_stack_outside_its_range_and_a_missing_number_are_refused() {
        assert!(read_file_spec("#> stack 0\nprint(1)\n").is_err());
        assert!(read_file_spec("#> stack 100001\nprint(1)\n").is_err());
        assert!(read_file_spec("#> seed\nprint(1)\n").is_err());
        assert!(read_file_spec("#> no-jit 3\nprint(1)\n").is_err());
    }
}

#[cfg(test)]
mod digit_separator_tests {
    use super::*;

    fn first(src: &str) -> TokenKind {
        Lexer::new(src).tokenize().unwrap()[0].kind.clone()
    }

    #[test]
    fn underscores_between_digits_are_ignored() {
        assert_eq!(first("1_000_000"), TokenKind::Int(1_000_000));
        assert_eq!(first("1__000"), TokenKind::Int(1000));
        assert_eq!(first("1_000.5"), TokenKind::Float(1000.5));
        assert_eq!(first("3.14_15"), TokenKind::Float(3.1415));
    }

    #[test]
    fn underscore_without_digit_after_is_not_part_of_the_number() {
        let toks = Lexer::new("1_").tokenize().unwrap();
        assert_eq!(toks[0].kind, TokenKind::Int(1));
        assert_eq!(toks[1].kind, TokenKind::Ident("_".into()));
    }
}

#[cfg(test)]
mod numeric_literal_tests {
    use super::*;

    fn first(src: &str) -> TokenKind {
        Lexer::new(src).tokenize().unwrap()[0].kind.clone()
    }

    fn fails(src: &str) -> bool {
        matches!(Lexer::new(src).tokenize(), Err(LexError::BadNumber { .. }))
    }

    #[test]
    fn hex_and_binary_literals() {
        assert_eq!(first("0xFF"), TokenKind::Int(255));
        assert_eq!(first("0xff_00"), TokenKind::Int(0xff00));
        assert_eq!(first("0b1010"), TokenKind::Int(10));
        assert_eq!(first("0b1111_0000"), TokenKind::Int(0xF0));
        assert_eq!(first("0x7FFFFFFFFFFFFFFF"), TokenKind::Int(i64::MAX));
    }

    #[test]
    fn bad_hex_and_binary_are_errors() {
        assert!(fails("0x"));
        assert!(fails("0b"));
        assert!(fails("0b102"));
        assert!(fails("0xG1"));
        assert!(fails("0xFFFFFFFFFFFFFFFFFF"));
    }

    #[test]
    fn exponent_makes_a_float() {
        assert_eq!(first("1e6"), TokenKind::Float(1_000_000.0));
        assert_eq!(first("2.5e-3"), TokenKind::Float(0.0025));
        assert_eq!(first("1E+2"), TokenKind::Float(100.0));
        assert_eq!(first("1_0e1_0"), TokenKind::Float(1e11));
    }

    #[test]
    fn e_without_digits_stays_a_name_as_before() {
        let toks = Lexer::new("1e").tokenize().unwrap();
        assert_eq!(toks[0].kind, TokenKind::Int(1));
        assert_eq!(toks[1].kind, TokenKind::Ident("e".into()));
    }

    #[test]
    fn too_big_numbers_are_errors_not_panics() {
        assert!(fails("9999999999999999999999"));
        assert!(fails("1e400"));
    }
}

#[cfg(test)]
mod double_colon_tests {
    use super::*;

    #[test]
    fn double_colon_is_one_token_and_single_colon_stays() {
        let toks = Lexer::new("int::max").tokenize().unwrap();
        assert_eq!(toks[1].kind, TokenKind::DoubleColon);
        let toks = Lexer::new("a: 1").tokenize().unwrap();
        assert_eq!(toks[1].kind, TokenKind::Colon);
    }
}
