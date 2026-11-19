//! tokens the Spaghetti (.spg) lexer recognizes. the vocabulary.

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // literals (the stuff people type)
    Int(i64),
    /// an integer literal above i64::MAX (up to u64::MAX): only valid where a u64 is expected
    UInt(u64),
    Float(f64),
    Str(String),
    Bool(bool),
    Ident(String),
    /// `@math`, `@str`, ... the name gets stored WITHOUT the `@`
    Namespace(String),

    /// `#http` (right after `attach`): a standard library. the name is stored WITHOUT the `#`
    StdLib(String),

    // keywords (reserved, hands off)
    Let,
    Const,
    Function,
    Return,
    If,
    Else,
    While,
    /// `do { } while (cond)`: the body runs once before the first check
    Do,
    For,
    In,
    Try,
    Catch,
    Throw,
    Break,
    Continue,
    When,
    Elif,
    Use,
    Attach,
    Public,
    From,
    Struct,
    NamespaceKw,
    Enum,
    /// `guard (me.field) { }` / `guard { }` inside a struct body
    Guard,

    // primitive types (type keywords, not plain idents)
    TypeInt,
    TypeI8,
    TypeI16,
    TypeI32,
    TypeU8,
    TypeU16,
    TypeU32,
    TypeU64,
    TypeFloat,
    TypeF32,
    TypeBool,
    TypeString,
    TypeVoid,
    TypeArray, // `array<T>`

    // operators (the little symbols)
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    StarStar,
    Eq,        // =
    EqEq,      // ==
    NotEq,     // !=
    PlusEq,    // +=
    MinusEq,   // -=
    StarEq,    // *=
    SlashEq,   // /=
    /// `%=`: the modulo compound assignment (`x %= 3`)
    PercentEq,
    /// `++`: increment, prefix or postfix (`++i`, `i++`), as an expression
    PlusPlus,
    /// `--`: decrement, prefix or postfix (`--i`, `i--`), as an expression
    MinusMinus,
    Lt,
    Gt,
    LtEq,
    GtEq,
    AndAnd,
    OrOr,
    /// `|>`: the pipe. `x |> f()` is `f(x)`. same as `then`.
    PipeArrow,
    /// `?`: marks an optional type (`int?`). a question mark with a job.
    Question,
    /// `??`: the default-value operator (`x ?? 0`). the fallback.
    QuestionQuestion,
    /// `none`: the absent value of an optional type. nothing, but typed.
    NoneLit,
    Not,
    Arrow,     // ->
    Colon,     // :
    DoubleColon, // `::`: type properties (`int::max`)
    Comma,
    Dot,
    DotDot, // `..` (ranges: `for i in 1..4`)

    // delimiters (brackets, commas, the usual)
    LParen,
    RParen,
    LBrace,
    RBrace,
    LAngle,    // < (also used in array<T>, disambiguated by the parser)
    RAngle,
    LBracket,  // [ — array literal, e.g. [1, 2, 3]
    RBracket,  // ]

    Semicolon,
    Newline,
    Eof,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub line: usize,
    pub col: usize,
    /// how many characters the token covers on its line (at least 1). the
    /// diagnostics use it to underline the exact spot, Python-style.
    pub len: usize,
}

impl Token {
    pub fn new(kind: TokenKind, line: usize, col: usize) -> Self {
        Token { kind, line, col, len: 1 }
    }

    pub fn spanned(kind: TokenKind, line: usize, col: usize, len: usize) -> Self {
        Token { kind, line, col, len: len.max(1) }
    }
}

/// resolves a word to a keyword or primitive type, if it's one.
/// returns None for a plain identifier (variable, function name, etc).
pub fn lookup_keyword(word: &str) -> Option<TokenKind> {
    match word {
        "let" => Some(TokenKind::Let),
        "const" => Some(TokenKind::Const),
        "fn" => Some(TokenKind::Function),
        "return" => Some(TokenKind::Return),
        "if" => Some(TokenKind::If),
        "else" => Some(TokenKind::Else),
        "while" => Some(TokenKind::While),
        "do" => Some(TokenKind::Do),
        "for" => Some(TokenKind::For),
        "in" => Some(TokenKind::In),
        "try" => Some(TokenKind::Try),
        "catch" => Some(TokenKind::Catch),
        "throw" => Some(TokenKind::Throw),
        "break" => Some(TokenKind::Break),
        "continue" => Some(TokenKind::Continue),
        "when" => Some(TokenKind::When),
        "elif" => Some(TokenKind::Elif),
        "then" => Some(TokenKind::PipeArrow), // verbose spelling of `|>`
        "and" => Some(TokenKind::AndAnd),
        "or" => Some(TokenKind::OrOr),
        "not" => Some(TokenKind::Not),
        "use" => Some(TokenKind::Use),
        "attach" => Some(TokenKind::Attach),
        "public" => Some(TokenKind::Public),
        "from" => Some(TokenKind::From),
        "struct" => Some(TokenKind::Struct),
        "namespace" => Some(TokenKind::NamespaceKw),
        "enum" => Some(TokenKind::Enum),
        "guard" => Some(TokenKind::Guard),
        "true" => Some(TokenKind::Bool(true)),
        "false" => Some(TokenKind::Bool(false)),
        "none" => Some(TokenKind::NoneLit),
        // `i64` is `int`: the same type, another name. same for `f64` and `float`
        "int" | "i64" => Some(TokenKind::TypeInt),
        "i8" => Some(TokenKind::TypeI8),
        "i16" => Some(TokenKind::TypeI16),
        "i32" => Some(TokenKind::TypeI32),
        "u8" => Some(TokenKind::TypeU8),
        "u16" => Some(TokenKind::TypeU16),
        "u32" => Some(TokenKind::TypeU32),
        "u64" => Some(TokenKind::TypeU64),
        "float" | "f64" => Some(TokenKind::TypeFloat),
        "f32" => Some(TokenKind::TypeF32),
        "bool" => Some(TokenKind::TypeBool),
        "string" => Some(TokenKind::TypeString),
        "void" => Some(TokenKind::TypeVoid),
        "array" => Some(TokenKind::TypeArray),
        _ => None,
    }
}

/// how a token reads to a person: `'('`, `end of file`, `name 'total'`, `number 5`...
/// for messages that say what the parser found instead of what it wanted.
pub fn describe(kind: &TokenKind) -> String {
    use TokenKind::*;
    match kind {
        Ident(name) => format!("name '{}'", name),
        Int(n) => format!("number {}", n),
        UInt(n) => format!("number {}", n),
        Float(f) => format!("number {}", f),
        Str(_) => "a string".to_string(),
        Bool(b) => format!("'{}'", b),
        Namespace(name) => format!("namespace '@{}'", name),
        StdLib(name) => format!("library '#{}'", name),
        Newline => "end of line".to_string(),
        Eof => "end of file".to_string(),
        LParen => "'('".to_string(),
        RParen => "')'".to_string(),
        LBrace => "'{'".to_string(),
        RBrace => "'}'".to_string(),
        LBracket => "'['".to_string(),
        RBracket => "']'".to_string(),
        Comma => "','".to_string(),
        Colon => "':'".to_string(),
        Dot => "'.'".to_string(),
        DotDot => "'..'".to_string(),
        Eq => "'='".to_string(),
        EqEq => "'=='".to_string(),
        NotEq => "'!='".to_string(),
        Arrow => "'->'".to_string(),
        Plus => "'+'".to_string(),
        Minus => "'-'".to_string(),
        Star => "'*'".to_string(),
        Slash => "'/'".to_string(),
        Percent => "'%'".to_string(),
        Lt => "'<'".to_string(),
        Gt => "'>'".to_string(),
        Not => "'!'".to_string(),
        Semicolon => "';'".to_string(),
        other => format!("{:?}", other).to_lowercase(),
    }
}

/// how many characters the token starting at column `col` of `line` covers: a name or a
/// number as a whole, a string up to its closing quote, a two-character operator as one.
/// the parser's errors don't carry a length, so the renderer measures it from the text.
pub fn token_extent(line: &str, col: usize) -> usize {
    let chars: Vec<char> = line.chars().collect();
    let start = col.saturating_sub(1);
    let Some(&first) = chars.get(start) else { return 1 };
    if first == '"' {
        let mut i = start + 1;
        while i < chars.len() {
            if chars[i] == '\\' {
                i += 2;
                continue;
            }
            if chars[i] == '"' {
                return i - start + 1;
            }
            i += 1;
        }
        return chars.len() - start;
    }
    if first.is_alphanumeric() || first == '_' {
        let mut i = start;
        while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '.' && first.is_ascii_digit()) {
            i += 1;
        }
        return (i - start).max(1);
    }
    const TWO: [&str; 15] = ["==", "!=", "<=", ">=", "->", "=>", "+=", "-=", "*=", "/=", "%=", "++", "--", "..", "&&"];
    if let Some(&second) = chars.get(start + 1) {
        let pair: String = [first, second].iter().collect();
        if TWO.contains(&pair.as_str()) || pair == "||" || pair == "??" || pair == "**" || pair == "|>" || pair == "::" {
            return 2;
        }
    }
    1
}
