//! Turns .spg source into a token stream.

use crate::token::{lookup_keyword, Token, TokenKind};
use std::fmt;

#[derive(Debug)]
pub enum LexError {
    UnexpectedChar { ch: char, line: usize, col: usize },
    UnterminatedString { line: usize, col: usize },
    UnterminatedBlockComment { line: usize, col: usize },
    /// `"${}"`: an interpolation with nothing inside the braces
    EmptyInterpolation { line: usize, col: usize },
    /// `@foo` where `foo` is not a builtin namespace (or a lone `@`)
    UnknownNamespace { name: String, line: usize, col: usize },
    /// `#> name` where `name` is not a known directive; `suggestion` is the
    /// closest known one, when it is a plausible typo
    UnknownDirective { name: String, suggestion: Option<&'static str>, line: usize, col: usize },
    /// the same directive written twice in one file
    DuplicateDirective { name: String, line: usize, col: usize },
    /// `#>` after the header (directives only exist at the top of a file)
    MisplacedDirective { line: usize, col: usize },
    /// a directive with a missing, wrong or unexpected argument
    BadDirectiveArgument { name: String, expected: &'static str, line: usize, col: usize },
}

/// every `#>` directive the engine understands
pub const DIRECTIVES: [&str; 3] = ["friendly-terminal", "timeout", "strict"];

/// what a file's `#>` header asked for
#[derive(Debug, Default, Clone, PartialEq)]
pub struct FileSpec {
    pub friendly_terminal: bool,
    /// `#> strict`: ambiguities become errors (int overflow, hidden names, dead code)
    pub strict: bool,
    /// `#> timeout N`: the program is stopped after N milliseconds
    pub timeout_ms: Option<u64>,
    /// every directive written, in order: (name, line, column)
    pub directives: Vec<(String, usize, usize)>,
}

impl fmt::Display for LexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LexError::UnexpectedChar { ch, line, col } => {
                write!(f, "unexpected character '{}' at line {}, column {}", ch, line, col)
            }
            LexError::EmptyInterpolation { line, col } => {
                write!(f, "empty interpolation in string, started at line {}, column {}", line, col)
            }
            LexError::UnterminatedString { line, col } => {
                write!(f, "unterminated string, started at line {}, column {}", line, col)
            }
            LexError::UnterminatedBlockComment { line, col } => {
                write!(f, "unterminated block comment, started at line {}, column {} (close it with //]])", line, col)
            }
            LexError::UnknownNamespace { name, line, col } => {
                let list: Vec<String> = crate::NAMESPACES.iter().map(|n| format!("@{}", n)).collect();
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
            LexError::EmptyInterpolation { line, col } => format!(
                "Oops! Empty `${{}}` in string in `{}` at line {}, column {}. Put an expression between the braces, or write `\\${{` for a literal dollar sign.",
                file, line, col
            ),
            LexError::UnterminatedBlockComment { line, col } => format!(
                "Oops! The block comment that starts at line {}, column {} in `{}` is never closed. Add `//]]` where it should end.",
                line, col, file
            ),
            LexError::UnterminatedString { line, col } => format!(
                "Oops! Unterminated string in `{}` at line {}, column {}. Check for a missing closing quote.",
                file, line, col
            ),
            LexError::UnknownNamespace { name, line, col } => {
                let list: Vec<String> = crate::NAMESPACES.iter().map(|n| format!("@{}", n)).collect();
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

/// the closest directive name to a typo, if it is at most two edits away
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

/// one piece of an interpolated string while it is being read
enum Part {
    Text(String),
    Expr(Vec<Token>),
}

/// tokenizes a piece of source that sits at a known position (an interpolated
/// expression, or a synthetic wrapper), so its tokens report real positions
fn fragment_tokens(src: &str, line: usize, col: usize) -> Result<Vec<Token>, LexError> {
    let mut lx = Lexer::new(src);
    lx.line = line;
    lx.col = col;
    lx.lex_tokens()
}

pub struct Lexer<'a> {
    /// decoded characters, NOT bytes: a multi-byte UTF-8 char (`á`, `ç`, `—`)
    /// is one element, one column, and survives into string literals intact
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

    /// tokens plus the file's `#>` header. Directives are read first and are
    /// not tokens: the parser never sees them
    pub fn tokenize_with_spec(mut self) -> Result<(Vec<Token>, FileSpec), LexError> {
        self.read_header()?;
        let mut tokens = self.lex_tokens()?;
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

            if ch == '"' {
                let parts = self.read_string_tokens()?;
                tokens.extend(parts);
                continue;
            }

            let kind = if ch.is_ascii_digit() {
                self.read_number()
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
    /// blank lines may sit between them)
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
        // `timeout` takes a whole number of milliseconds; the others take nothing
        while matches!(self.peek(), Some(' ' | '\t')) {
            self.advance();
        }
        let mut timeout_ms = None;
        if name == "timeout" {
            let mut digits = String::new();
            while let Some(c) = self.peek() {
                if c.is_ascii_digit() {
                    digits.push(c);
                    self.advance();
                } else {
                    break;
                }
            }
            match digits.parse::<u64>() {
                Ok(n) if n > 0 => timeout_ms = Some(n),
                _ => {
                    return Err(LexError::BadDirectiveArgument {
                        name,
                        expected: "a whole number of milliseconds greater than 0, like `#> timeout 5000`",
                        line,
                        col,
                    })
                }
            }
        }
        // a directive owns its whole line: only spaces or a comment may follow
        while matches!(self.peek(), Some(' ' | '\t' | '\r')) {
            self.advance();
        }
        match self.peek() {
            None | Some('\n') => {}
            Some('/') if self.peek_next() == Some('/') => {}
            Some(_) => {
                let expected = if name == "timeout" {
                    "just the number of milliseconds, nothing else on the line"
                } else {
                    "no argument"
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
            _ => self.spec.timeout_ms = timeout_ms,
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
                // block comment: //[[ ... //]]  (no nesting; the first //]] closes it)
                Some('/') if self.at(0, "//[[") => {
                    let (line, col) = (self.line, self.col);
                    for _ in 0..4 {
                        self.advance();
                    }
                    loop {
                        if self.peek().is_none() {
                            return Err(LexError::UnterminatedBlockComment { line, col });
                        }
                        if self.at(0, "//]]") {
                            for _ in 0..4 {
                                self.advance();
                            }
                            break;
                        }
                        self.advance();
                    }
                }
                Some('/') if self.peek_next() == Some('/') => {
                    // line comment: // ...
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

    fn read_number(&mut self) -> TokenKind {
        let start = self.pos;
        let mut is_float = false;

        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                self.advance();
            } else if c == '.' && !is_float && self.peek_next().map_or(false, |n| n.is_ascii_digit()) {
                is_float = true;
                self.advance();
            } else {
                break;
            }
        }

        let text: String = self.source[start..self.pos].iter().collect();
        if is_float {
            TokenKind::Float(text.parse().unwrap())
        } else {
            TokenKind::Int(text.parse().unwrap())
        }
    }

    /// `"..."`: a normal string. Escapes work as before, and `${expr}`
    /// interpolates. The literal becomes a parenthesized `+` chain of string
    /// pieces and `@into.string(expr)` calls, so the parser and typechecker
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
    /// consumed). Nested braces, strings and raw strings are copied whole, so
    /// `"${@str.len("}")}"` closes at the right brace.
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

    /// the rest of a `"..."` that sits inside `${...}`, copied raw through its
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

    /// `'...'`: a RAW string. Everything up to the next `'` is the content,
    /// byte for byte: no escapes at all (`'a\nb'` is the 4 characters
    /// `a`, `\`, `n`, `b`), and a `"` inside is just a `"`. Because there is
    /// no escape, a `'` can never appear inside one: the first `'` after
    /// the opening one always closes it. That is deliberate: the lexer stays
    /// stuck in the string it already opened and never looks ahead for a
    /// "better" closing quote, so `'print('hi')'` fails instead of silently
    /// gluing strings together.
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

    /// `@name` → `TokenKind::Namespace(name)`; anything that is not one of
    /// `NAMESPACES` is a lex error (so a typo like `@mth` is caught here)
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
            '%' => TokenKind::Percent,
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
            ':' => TokenKind::Colon,
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
        // the old spellings are plain identifiers now
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
        // `é` is 2 bytes; the `x` after it must still be column 8
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
        // backslash-n stays as two characters; backslash does not hide the closing quote
        assert_eq!(kinds("'a\\nb'")[0], TokenKind::Str("a\\nb".into()));
        assert_eq!(kinds("'a\\'")[0], TokenKind::Str("a\\".into()));
    }

    #[test]
    fn raw_string_closes_at_the_first_quote_and_never_looks_ahead() {
        // `'print('` is already a complete string; `hi` is a plain identifier;
        // `')'` is another string. The lexer never skips ahead to a "better"
        // closing quote: the parser is the one that rejects this sequence.
        assert_eq!(
            kinds("'print('hi')'"),
            vec![
                TokenKind::Str("print(".into()),
                TokenKind::Ident("hi".into()),
                TokenKind::Str(")".into()),
                TokenKind::Eof
            ]
        );
        // with an odd number of quotes the last one is left open
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

/// the directive header of a file, read on its own: the CLI uses it to decide
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
        let src = "print(1)\n//[[ print(2)\n  // inner line comment, no nesting //[[\n//]]\nprint(3) //[[ tail //]] print(4)";
        let toks: Vec<TokenKind> = Lexer::new(src).tokenize().unwrap().into_iter().map(|t| t.kind).collect();
        let text = format!("{:?}", toks);
        assert!(text.contains("Int(1)") && text.contains("Int(3)") && text.contains("Int(4)"));
        assert!(!text.contains("Int(2)"), "{}", text);
    }

    #[test]
    fn unterminated_block_comment_is_an_error_with_its_start() {
        match Lexer::new("print(1)\n  //[[ never closed\nprint(2)").tokenize() {
            Err(LexError::UnterminatedBlockComment { line, col }) => assert_eq!((line, col), (2, 3)),
            other => panic!("expected UnterminatedBlockComment, got {:?}", other),
        }
    }

    #[test]
    fn a_closing_marker_alone_is_just_a_line_comment() {
        let toks: Vec<TokenKind> = Lexer::new("print(1) //]] nothing to close\nprint(2)").tokenize().unwrap().into_iter().map(|t| t.kind).collect();
        let text = format!("{:?}", toks);
        assert!(text.contains("Int(2)"), "{}", text);
    }
}
