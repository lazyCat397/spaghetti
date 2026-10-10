//! Spaghetti's AST.
//!
//! `Expr` always produces a value; `Stmt` usually doesn't. `if` exists as
//! an `Expr` variant too — when used in statement position the parser
//! just wraps it in `Stmt::ExprStmt`, but the "else is mandatory, both
//! branches share a type" rule only applies when `if` is in expression
//! position (e.g. the right side of a `let`); the typechecker enforces
//! that.

/// Separator between a module's unique tag and a symbol's own name in a
/// mangled name (`<tag>::<name>`). Identifiers can only contain letters,
/// digits and `_`, so this can never occur in a name the user wrote —
/// which is what makes `display_name` unambiguous without any lookup
/// table. The module resolver builds mangled names with it; diagnostics
/// undo it. Keeping both halves next to each other is the point.
pub const MANGLE_SEP: &str = "::";

/// What the user actually wrote: strips the module tag from a mangled
/// name (`errmod#c29b::Ponto` -> `Ponto`), leaves ordinary names alone.
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
    /// which file this position is in. The parser itself never sets
    /// this — it only sees a token stream, not a filename — so every
    /// `Pos` starts out `None`. spg-modules (the only thing that ever
    /// runs a parser in this codebase) stamps every `Pos` with its
    /// file right after parsing, single-file programs included, so in
    /// practice this is always `Some` by the time anything reads it.
    pub file: Option<String>,
}

// type annotations — `int`, `array<int>`, a custom struct name, etc.

#[derive(Debug, Clone, PartialEq)]
pub enum TypeExpr {
    Int,
    Float,
    Bool,
    String,
    Void,
    Array(Box<TypeExpr>),
    /// `map<K, V>` — `map` is only a type name when followed by `<` (contextual,
    /// so a variable called `map` keeps working)
    Map(Box<TypeExpr>, Box<TypeExpr>),
    Named(String), // a user-defined struct
}

// expressions — always produce a value

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    IntLit(i64, Pos),
    FloatLit(f64, Pos),
    StringLit(String, Pos),
    BoolLit(bool, Pos),
    ArrayLit(Vec<Expr>, Pos),
    /// `["a": 1, "b": 2]`, and `[:]` for the empty map (no entries)
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

    /// `arr[index]` — reading an element out of an array
    Index {
        array: Box<Expr>,
        index: Box<Expr>,
        pos: Pos,
    },

    /// `Name { field: value, ... }` — constructs a struct value. Only
    /// parsed in "struct-literal-allowed" position (see the parser's
    /// `no_struct_literal` flag) — in an `if`/`while` condition or a
    /// `for .. in` iterable, `Name {` would otherwise be ambiguous with
    /// the following block's opening brace.
    StructLit {
        name: String,
        fields: Vec<(String, Expr)>,
        pos: Pos,
    },

    /// `obj.field` — reading a struct field
    Field {
        object: Box<Expr>,
        field: String,
        pos: Pos,
    },

    /// `if` as an EXPRESSION: `else` is required here (checked by the
    /// parser itself, not just the typechecker), and both branches must
    /// agree on type — that part the typechecker validates.
    IfExpr {
        cond: Box<Expr>,
        then_branch: Box<Block>,
        else_branch: Box<Block>, // required, not Option
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
}

#[derive(Debug, Clone, PartialEq)]
pub enum UnaryOp {
    Neg,
    Not,
}

// statements — control flow and declarations, no value produced

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub pos: Pos,
}

/// The value of an enum variant (`enum Status { Ok = 200 }`): a literal of one
/// of the four plain types. Every variant of one enum has the same kind.
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
    /// `let x = 10` or `let x: int = 10` — type_ann is None when inferred
    Let {
        name: String,
        type_ann: Option<TypeExpr>,
        value: Expr,
        exported: bool, // true if preceded by `export`
        pos: Pos,
    },

    /// `const name: string = "spaghetti"`
    Const {
        name: String,
        type_ann: Option<TypeExpr>,
        value: Expr,
        exported: bool, // true if preceded by `export`
        pos: Pos,
    },

    /// `if` as a plain STATEMENT — else is optional here, unlike IfExpr
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

    /// `for i in 1..4 { }` / `for i in 0..10 step 2 { }` — counts without
    /// building an array. End is EXCLUSIVE (like Python's `range`); a
    /// negative step counts down. `step` is a contextual word, not a
    /// keyword: it is only read right after the end expression.
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

    /// `break` — exits the nearest enclosing while/for-in loop.
    /// Rejected by the typechecker outside of a loop.
    Break { pos: Pos },

    /// `continue` — skips to the next iteration of the nearest
    /// enclosing loop. Rejected outside of a loop, same as `break`.
    Continue { pos: Pos },

    /// `throw "message"` — raises an exception carrying a string; the nearest
    /// enclosing `try`/`catch` (in this function or any caller) catches it
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
        /// `None` when the trailing `-> Type` was omitted — the
        /// typechecker infers it from the body's return statements
        /// instead of requiring it up front.
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

    /// `enum Color { Red, Green, Blue }` — a closed set of named options.
    /// A distinct type (not an `int`): only `==` / `!=` apply to it. Each
    /// variant carries its own position so a duplicate can be reported
    /// where it was written.
    EnumDecl {
        name: String,
        variants: Vec<String>,
        /// one resolved value per variant, same order as `variants`. The parser
        /// fills in the automatic numbering (0, 1, 2...), so everything after it
        /// sees a value for every variant. All of them share one type.
        values: Vec<EnumValue>,
        exported: bool,
        pos: Pos,
    },

    Import {
        names: Vec<String>,
        from: String,
        pos: Pos,
    },

    /// `install "./file.spg"` — brings in EVERY exported name of that file,
    /// without listing them. Consumed by spg-modules, like `Import`.
    Install {
        from: String,
        pos: Pos,
    },

    /// `export { a, b } from "./y.spg"` — makes names that live in
    /// another module importable FROM THIS ONE, without declaring
    /// anything here. Deliberately not the same as `Import`: an import
    /// only brings a name into this file's own scope, and never makes it
    /// importable onward. Consumed by spg-modules (like `Import`), so it
    /// never reaches the typechecker or compiler.
    ReExport {
        names: Vec<String>,
        from: String,
        pos: Pos,
    },


    /// `x = value` — reassigns an already-declared `let`. Distinct from
    /// `Let`/`Const`: nothing new is declared, just an existing slot
    /// updated. Checking that the target isn't `const` is the
    /// typechecker's job — the parser doesn't track mutability.
    Assign {
        name: String,
        value: Expr,
        pos: Pos,
    },

    /// `arr[index] = value` — mutates an existing array element in
    /// place. Kept distinct from `Assign` (whole-variable rebind)
    /// rather than generalizing `Assign`'s target, since the two compile
    /// to entirely different opcodes (SetLocal vs IndexSet) and only
    /// array element assignment needs an index expression at all.
    IndexAssign {
        array: Expr,
        index: Expr,
        value: Expr,
        pos: Pos,
    },

    /// `arr[index] OP= rhs` (+=, -=, *=, /=). Kept as its own node
    /// instead of desugaring to `arr[index] = arr[index] OP rhs` in the
    /// parser: that desugaring would duplicate the `array`/`index`
    /// subexpressions, so anything with a side effect inside the index
    /// (e.g. `arr[next()] += 1`) would run twice. This node carries
    /// `array`/`index` exactly once; spg-compiler evaluates each a
    /// single time and reuses them for both the read and the write.
    IndexCompoundAssign {
        array: Expr,
        index: Expr,
        op: BinaryOp,
        value: Expr,
        pos: Pos,
    },

    /// `obj.field = value` — mutates an existing struct field in place.
    /// Same shape and same reasoning as `IndexAssign`, field name instead
    /// of an index expression (so nothing to duplicate/evaluate-once
    /// worry about — a field name isn't an expression).
    FieldAssign {
        object: Expr,
        field: String,
        value: Expr,
        pos: Pos,
    },

    /// `obj.field OP= rhs` — same relationship to FieldAssign that
    /// IndexCompoundAssign has to IndexAssign, minus the "evaluate once"
    /// concern (a field name has nothing to re-evaluate), kept as its
    /// own node anyway so spg-compiler's dispatch mirrors the index case
    /// exactly and object is still only compiled once.
    FieldCompoundAssign {
        object: Expr,
        field: String,
        op: BinaryOp,
        value: Expr,
        pos: Pos,
    },

    /// an expression used as a statement (a bare function call, or an
    /// if-expression whose value is discarded)
    ExprStmt(Expr),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: String,
    pub type_ann: TypeExpr, // always required in a signature/field
}

#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub stmts: Vec<Stmt>,
}
