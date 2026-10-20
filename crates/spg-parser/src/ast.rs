//! Spaghetti's AST. the family tree.
//!
//! `Expr` always produces a value. `Stmt` usually doesn't. `if` exists as
//! an `Expr` variant too. in statement position the parser
//! just wraps it in `Stmt::ExprStmt`. but the "else is mandatory, both
//! branches share a type" rule only applies in expression
//! position (e.g. the right side of a `let`). the typechecker enforces
//! that. yes, it's strict.

/// separator between a module's unique tag and a symbol's own name in a
/// mangled name (`<tag>::<name>`). identifiers can only contain letters,
/// digits and `_`, so this can never show up in a name the user wrote.
/// that's what makes `display_name` unambiguous, no lookup needed.
/// table. the module resolver builds mangled names with it. diagnostics
/// undo it. keeping both halves next to each other is the whole point.
pub const MANGLE_SEP: &str = "::";

/// what the user actually wrote: strips the module tag from a mangled
/// name (`errmod#c29b::Ponto` -> `Ponto`). ordinary names: left alone.
pub fn display_name(name: &str) -> &str {
    match name.rfind(MANGLE_SEP) {
        Some(i) => &name[i + MANGLE_SEP.len()..],
        None => name,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Pos {
    pub line: usize,
    pub col: usize,
    /// which file this position is in. the parser itself never sets
    /// this (it only sees tokens, not filenames), so every
    /// `Pos` starts out `None`. spg-modules (the only thing that runs
    /// a parser in this codebase) stamps every `Pos` with its
    /// file right after parsing, single-file programs included, so in
    /// practice it's always `Some` by the time anything reads it.
    pub file: Option<String>,
}

// type annotations: `int`, `array<int>`, a custom struct name, etc.

#[derive(Debug, Clone, PartialEq)]
pub enum TypeExpr {
    Int,
    Float,
    Bool,
    String,
    Void,
    Array(Box<TypeExpr>),
    /// `map<K, V>`. `map` is only a type name when followed by `<` (contextual,
    /// so a variable called `map` keeps working. nice of it.
    Map(Box<TypeExpr>, Box<TypeExpr>),
    Named(String), // a user-defined struct
    /// `function(int, string) -> bool`: the type of a function value
    Function { params: Vec<TypeExpr>, ret: Box<TypeExpr> },
    /// `T?`: a value of type T, or `none`. the question mark again.
    Optional(Box<TypeExpr>),
    /// `task<T>`: the result of a `-> task<T>` function. a `task` only means
    /// this when followed by `<` (contextual, like `map`). the body runs at call time.
    Task(Box<TypeExpr>),
}

// expressions. always produce a value

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    IntLit(i64, Pos),
    FloatLit(f64, Pos),
    StringLit(String, Pos),
    BoolLit(bool, Pos),
    /// `none`: the absent value of an optional type
    NoneLit(Pos),
    ArrayLit(Vec<Expr>, Pos),
    /// `["a": 1, "b": 2]`, and `[:]` for the empty map (no entries). lonely map.
    MapLit(Vec<(Expr, Expr)>, Pos),

    Ident(String, Pos),

    Binary {
        left: Box<Expr>,
        op: BinaryOp,
        right: Box<Expr>,
        pos: Pos,
    },

    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
        pos: Pos,
    },

    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
        pos: Pos,
    },

    /// `arr[index]`: reading an element out of an array
    Index {
        array: Box<Expr>,
        index: Box<Expr>,
        pos: Pos,
    },

    /// `Name { field: value, ... }`: builds a struct value. only
    /// parsed in "struct-literal-allowed" position (see the parser's
    /// `no_struct_literal` flag). in an `if`/`while` condition or a
    /// `for .. in` iterable, `Name {` would be ambiguous with
    /// the following block's opening brace. the classic trap.
    StructLit {
        name: String,
        fields: Vec<(String, Expr)>,
        pos: Pos,
    },

    /// `obj.field`: reading a struct field
    Field {
        object: Box<Expr>,
        field: String,
        pos: Pos,
    },

    /// `if` as an EXPRESSION: `else` is required here (checked by the
    /// parser itself, not just the typechecker), and both branches must
    /// agree on type. that part the typechecker validates.
    IfExpr {
        cond: Box<Expr>,
        then_branch: Box<Block>,
        else_branch: Box<Block>, // required, not Option
        pos: Pos,
    },
    /// `function(x: int) -> int { ... }` in expression position: an anonymous
    /// function value. without `-> T` the return type is void
    Lambda {
        params: Vec<Param>,
        return_type: Option<TypeExpr>,
        body: Block,
        pos: Pos,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Eq,
    NotEq,
    Lt,
    Gt,
    LtEq,
    GtEq,
    And,
    Or,
    /// `a ?? b`: `a` unless it's `none`, else `b` (the checker makes it its own node)
    Coalesce,
}

#[derive(Debug, Clone, PartialEq)]
pub enum UnaryOp {
    Neg,
    Not,
}

// statements. control flow and declarations, no value produced

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub pos: Pos,
}

/// the value of an enum variant (`enum Status { Ok = 200 }`): a literal of one
/// of the four plain types. every variant of one enum has the same kind.
#[derive(Debug, Clone, PartialEq)]
pub enum EnumValue {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
}

impl EnumValue {
    /// the name of the value's type, as written in the language
    pub fn type_name(&self) -> &'static str {
        match self {
            EnumValue::Int(_) => "int",
            EnumValue::Float(_) => "float",
            EnumValue::Str(_) => "string",
            EnumValue::Bool(_) => "bool",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    /// `let x = 10` or `let x: int = 10`. type_ann is None when inferred
    Let {
        name: String,
        type_ann: Option<TypeExpr>,
        value: Expr,
        exported: bool, // true if preceded by `export`
        pos: Pos,
    },

    /// `const name: string = "spaghetti"`. the immovable one.
    Const {
        name: String,
        type_ann: Option<TypeExpr>,
        value: Expr,
        exported: bool, // true if preceded by `export`
        pos: Pos,
    },

    /// `when (subject) { 1, 2 { } 3 { } else { } }`: a match on an int, a string
    /// or an enum. lowered by the checker to an if/elif chain on a hidden local.
    Match {
        subject: Expr,
        arms: Vec<MatchArm>,
        else_branch: Option<Block>,
        pos: Pos,
    },

    /// `if` as a plain STATEMENT. else is optional here, unlike IfExpr
    IfStmt {
        cond: Expr,
        then_branch: Block,
        else_branch: Option<Block>,
        pos: Pos,
    },

    While {
        cond: Expr,
        body: Block,
        pos: Pos,
    },

    ForIn {
        var_name: String,
        iterable: Expr,
        body: Block,
        pos: Pos,
    },

    /// `for i in 1..4 { }` / `for i in 0..10 step 2 { }`. counts without
    /// building an array. end is EXCLUSIVE (like Python's `range`). a
    /// negative step counts down. `step` is a contextual word, not a
    /// keyword. it's only read right after the end expression.
    ForRange {
        var_name: String,
        start: Expr,
        end: Expr,
        step: Option<Expr>,
        body: Block,
        pos: Pos,
    },

    Return {
        value: Option<Expr>,
        pos: Pos,
    },

    /// `break`: exits the nearest enclosing while/for-in loop.
    /// rejected by the typechecker outside of a loop.
    Break { pos: Pos },

    /// `continue`: skips to the next iteration of the nearest
    /// enclosing loop. rejected outside a loop, same as `break`.
    Continue { pos: Pos },

    /// `throw "message"`: raises an exception carrying a string. the nearest
    /// enclosing `try`/`catch` (this function or any caller) catches it.
    Throw { value: Expr, pos: Pos },

    Try {
        try_block: Block,
        catch_var: String,
        catch_type: Option<TypeExpr>, // `catch (e: Error)`
        catch_block: Block,
        pos: Pos,
    },

    FunctionDecl {
        name: String,
        params: Vec<Param>,
        /// `None` when the trailing `-> Type` was omitted. the
        /// typechecker infers it from the body's return statements
        /// instead of demanding it up front. chill.
        return_type: Option<TypeExpr>,
        body: Block,
        exported: bool, // true if preceded by `export`
        pos: Pos,
    },

    StructDecl {
        name: String,
        fields: Vec<Param>, // reuses Param: name + type
        exported: bool,
        pos: Pos,
    },

    /// `enum Color { Red, Green, Blue }`: a closed set of named options.
    /// a distinct type (not an `int`). only `==` / `!=` apply to it. each
    /// variant carries its own position, so a duplicate can be reported
    /// where it was written. shame.
    EnumDecl {
        name: String,
        variants: Vec<String>,
        /// one resolved value per variant, same order as `variants`. the parser
        /// fills in the automatic numbering (0, 1, 2...), so everything after it
        /// sees a value for every variant. all of them share one type.
        values: Vec<EnumValue>,
        exported: bool,
        pos: Pos,
    },

    Import {
        names: Vec<String>,
        from: String,
        pos: Pos,
    },

    /// `install "./file.spg"`: brings in EVERY exported name of that file,
    /// without listing them. consumed by spg-modules, like `Import`.
    Install {
        from: String,
        pos: Pos,
        /// `install #name`: `from` is a standard module name
        /// (looked up in the standard library folder), not a path
        std: bool,
    },

    /// `export { a, b } from "./y.spg"`: makes names that live in
    /// another module importable FROM THIS ONE, without declaring
    /// anything here. deliberately not the same as `Import`: an import
    /// only brings a name into this file's own scope, never makes it
    /// importable onward. consumed by spg-modules (like `Import`), so it
    /// never reaches the typechecker or compiler. go away, typechecker.
    ReExport {
        names: Vec<String>,
        from: String,
        pos: Pos,
    },


    /// `x = value`: reassigns an already-declared `let`. distinct from
    /// `Let`/`Const`: nothing new is declared, just an existing slot
    /// updated. checking that the target isn't `const` is the
    /// typechecker's job. the parser doesn't track mutability.
    Assign {
        name: String,
        value: Expr,
        pos: Pos,
    },

    /// `arr[index] = value`: mutates an existing array element in
    /// place. kept distinct from `Assign` (whole-variable rebind)
    /// rather than generalizing `Assign`'s target, since the two compile
    /// to totally different opcodes (SetLocal vs IndexSet), and only
    /// array element assignment needs an index expression at all.
    IndexAssign {
        array: Expr,
        index: Expr,
        value: Expr,
        pos: Pos,
    },

    /// `arr[index] OP= rhs` (+=, -=, *=, /=). kept as its own node
    /// instead of desugaring to `arr[index] = arr[index] OP rhs` in the
    /// parser. that desugaring would duplicate the `array`/`index`
    /// subexpressions, so anything with a side effect in the index
    /// (e.g. `arr[next()] += 1`) would run twice. this node carries
    /// `array`/`index` exactly once. spg-compiler evaluates each one
    /// a single time and reuses them for the read and the write. efficient.
    IndexCompoundAssign {
        array: Expr,
        index: Expr,
        op: BinaryOp,
        value: Expr,
        pos: Pos,
    },

    /// `obj.field = value`: mutates an existing struct field in place.
    /// same shape and reasoning as `IndexAssign`, field name instead
    /// of an index expression (so nothing to duplicate or evaluate-once
    /// worry about. a field name isn't an expression.
    FieldAssign {
        object: Expr,
        field: String,
        value: Expr,
        pos: Pos,
    },

    /// `obj.field OP= rhs`: same relationship to FieldAssign that
    /// IndexCompoundAssign has to IndexAssign, minus the "evaluate once"
    /// concern (a field name has nothing to re-evaluate). kept as its
    /// own node anyway, so spg-compiler's dispatch mirrors the index case
    /// exactly, and object still gets compiled only once.
    FieldCompoundAssign {
        object: Expr,
        field: String,
        op: BinaryOp,
        value: Expr,
        pos: Pos,
    },

    /// an expression used as a statement (a bare function call, or an
    /// if-expression whose value gets thrown away)
    ExprStmt(Expr),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: String,
    pub type_ann: TypeExpr, // always required in a signature/field
    /// `n: int = 10`: a literal used when the caller leaves this arg out.
    /// only literals (`10`, `-1.5`, `"x"`, `true`), so the value needs no scope
    /// of its own. only named functions and methods may have one.
    pub default: Option<Expr>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub stmts: Vec<Stmt>,
    /// native libraries enabled by `install #...` (e.g. "powershell")
    pub natives: Vec<String>,
}

impl Expr {
    /// where the expression starts, when the variant carries a position
    pub fn pos(&self) -> Option<&Pos> {
        match self {
            Expr::IntLit(_, p)
            | Expr::FloatLit(_, p)
            | Expr::StringLit(_, p)
            | Expr::BoolLit(_, p)
            | Expr::NoneLit(p)
            | Expr::ArrayLit(_, p)
            | Expr::MapLit(_, p)
            | Expr::Ident(_, p) => Some(p),
            Expr::Binary { pos, .. }
            | Expr::Unary { pos, .. }
            | Expr::Call { pos, .. }
            | Expr::Index { pos, .. }
            | Expr::StructLit { pos, .. }
            | Expr::Field { pos, .. }
            | Expr::IfExpr { pos, .. }
            | Expr::Lambda { pos, .. } => Some(pos),
        }
    }
}

/// one `values { body }` arm of a `when` match: `"up", "w" { ... }`
#[derive(Debug, Clone, PartialEq)]
pub struct MatchArm {
    pub values: Vec<Expr>,
    pub body: Block,
    pub pos: Pos,
}

impl Stmt {
    /// where the statement starts (an expression statement takes its expression's position)
    pub fn pos(&self) -> Option<&Pos> {
        match self {
            Stmt::Let { pos, .. }
            | Stmt::Const { pos, .. }
            | Stmt::IfStmt { pos, .. }
            | Stmt::Match { pos, .. }
            | Stmt::While { pos, .. }
            | Stmt::ForIn { pos, .. }
            | Stmt::ForRange { pos, .. }
            | Stmt::Return { pos, .. }
            | Stmt::Break { pos }
            | Stmt::Continue { pos }
            | Stmt::Throw { pos, .. }
            | Stmt::Try { pos, .. }
            | Stmt::FunctionDecl { pos, .. }
            | Stmt::StructDecl { pos, .. }
            | Stmt::EnumDecl { pos, .. }
            | Stmt::Import { pos, .. }
            | Stmt::Install { pos, .. }
            | Stmt::ReExport { pos, .. }
            | Stmt::Assign { pos, .. }
            | Stmt::IndexAssign { pos, .. }
            | Stmt::IndexCompoundAssign { pos, .. }
            | Stmt::FieldAssign { pos, .. }
            | Stmt::FieldCompoundAssign { pos, .. } => Some(pos),
            Stmt::ExprStmt(e) => e.pos(),
        }
    }
}
