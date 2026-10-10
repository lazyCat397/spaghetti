//! Tokens recognized by the Spaghetti (.spg) lexer

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // literals
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    Ident(String),
    /// `@math`, `@str`, ... — the name is stored WITHOUT the `@`
    Namespace(String),

    // keywords
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
    Import,
    Install,
    Export,
    From,
    Struct,
    Enum,

    // primitive types (recognized as type keywords, not plain idents)
    TypeInt,
    TypeFloat,
    TypeBool,
    TypeString,
    TypeVoid,
    TypeArray, // `array<T>`

    // operators
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
    Not,
    Arrow,     // ->
    Colon,     // :
    Comma,
    Dot,
    DotDot, // `..` (ranges: `for i in 1..4`)

    // delimiters
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

/// Resolves a word to a keyword or primitive type, if it's one.
/// Returns None for a plain identifier (variable, function name, etc).
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
        "import" => Some(TokenKind::Import),
        "install" => Some(TokenKind::Install),
        "export" => Some(TokenKind::Export),
        "from" => Some(TokenKind::From),
        "struct" => Some(TokenKind::Struct),
        "enum" => Some(TokenKind::Enum),
        "true" => Some(TokenKind::Bool(true)),
        "false" => Some(TokenKind::Bool(false)),
        "int" => Some(TokenKind::TypeInt),
        "float" => Some(TokenKind::TypeFloat),
        "bool" => Some(TokenKind::TypeBool),
        "string" => Some(TokenKind::TypeString),
        "void" => Some(TokenKind::TypeVoid),
        "array" => Some(TokenKind::TypeArray),
        _ => None,
    }
}
