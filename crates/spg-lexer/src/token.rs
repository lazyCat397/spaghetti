//! tokens the Spaghetti (.spg) lexer recognizes. the vocabulary.

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // literals (the stuff people type)
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    Ident(String),
    /// `@math`, `@str`, ... the name gets stored WITHOUT the `@`
    Namespace(String),

    /// `#http` (right after `install`): a standard library. the name is stored WITHOUT the `#`
    StdLib(String),

    // keywords (reserved, hands off)
    Let,
    Const,
    Function,
    Return,
    If,
    Else,
    While,
    For,
    In,
    Try,
    Catch,
    Throw,
    Break,
    Continue,
    When,
    Elif,
    Import,
    Install,
    Export,
    From,
    Struct,
    Enum,

    // primitive types (type keywords, not plain idents)
    TypeInt,
    TypeFloat,
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
}

impl Token {
    pub fn new(kind: TokenKind, line: usize, col: usize) -> Self {
        Token { kind, line, col }
    }
}

/// resolves a word to a keyword or primitive type, if it's one.
/// returns None for a plain identifier (variable, function name, etc).
pub fn lookup_keyword(word: &str) -> Option<TokenKind> {
    match word {
        "let" => Some(TokenKind::Let),
        "const" => Some(TokenKind::Const),
        "function" => Some(TokenKind::Function),
        "return" => Some(TokenKind::Return),
        "if" => Some(TokenKind::If),
        "else" => Some(TokenKind::Else),
        "while" => Some(TokenKind::While),
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
        "import" => Some(TokenKind::Import),
        "install" => Some(TokenKind::Install),
        "export" => Some(TokenKind::Export),
        "from" => Some(TokenKind::From),
        "struct" => Some(TokenKind::Struct),
        "enum" => Some(TokenKind::Enum),
        "true" => Some(TokenKind::Bool(true)),
        "false" => Some(TokenKind::Bool(false)),
        "none" => Some(TokenKind::NoneLit),
        "int" => Some(TokenKind::TypeInt),
        "float" => Some(TokenKind::TypeFloat),
        "bool" => Some(TokenKind::TypeBool),
        "string" => Some(TokenKind::TypeString),
        "void" => Some(TokenKind::TypeVoid),
        "array" => Some(TokenKind::TypeArray),
        _ => None,
    }
}
