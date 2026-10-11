//! Resolved types and the typed AST produced by the typechecker.
//!
//! `Ty` is the checker's internal, comparable representation of a type —
//! distinct from `TypeExpr` (spg_parser::ast), which is just the syntax the
//! user wrote. Every expression node in `TypedProgram` carries a `Ty`
//! resolved by the checker, so `spg-compiler` never has to re-derive types.

use spg_parser::ast::{BinaryOp, Param, UnaryOp};

use crate::builtins::BuiltinId;

#[derive(Debug, Clone)]
pub enum Ty {
    Int,
    Float,
    Bool,
    String,
    Void,
    Array(Box<Ty>),
    /// `map<K, V>`. `Map(Void, Void)` is the type of the literal `[:]` (see `is_empty_map`)
    Map(Box<Ty>, Box<Ty>),
    Named(String), // struct
    /// accepts any type when used as a parameter type — reserved for
    /// builtins in v1.0 (e.g. print), not declarable by user code, so it
    /// doesn't become disguised dynamic typing throughout the language
    Any,
    /// accepts `array<T>` for any element type T when used as a builtin
    /// parameter type — lets array builtins (len, push, reverse, ...) be
    /// polymorphic over element type without becoming user-facing
    /// generics. Not declarable by user code, same spirit as `Any`.
    AnyArray,
    /// accepts `map<K, V>` for any K and V when used as a builtin parameter
    /// type (`@map.*`). Not declarable by user code, same spirit as `AnyArray`.
    AnyMap,
    /// `function(P1, P2) -> R`: the type of a function value (a named function,
    /// a lambda, or a variable holding either)
    Function { params: Vec<Ty>, ret: Box<Ty> },
    /// `T?`: a value of type T, or `none`
    Optional(Box<Ty>),
    /// the type of the literal `none` on its own; it fits every optional type
    NoneLit,
}

impl Ty {
    /// whether a value of type `found` may be stored where `self` is expected:
    /// the same type, or (for `T?`) a `T`, a `T?`, or `none`. Never the other
    /// way round: a `T?` does not fit a `T`
    pub fn accepts(&self, found: &Ty) -> bool {
        if self == found {
            return true;
        }
        match self {
            Ty::Optional(inner) => {
                matches!(found, Ty::NoneLit)
                    || inner.as_ref() == found
                    || matches!(found, Ty::Optional(other) if other == inner)
            }
            _ => false,
        }
    }

    /// the type of the literal `[:]`: a map with no known key/value type. It is
    /// written as `Map(Void, Void)` (a user cannot write `map<void, void>`).
    pub fn is_empty_array(&self) -> bool {
        matches!(self, Ty::Array(inner) if matches!(**inner, Ty::Void))
    }

    pub fn is_empty_map(&self) -> bool {
        matches!(self, Ty::Map(k, v) if matches!(**k, Ty::Void) && matches!(**v, Ty::Void))
    }
}

/// Structural equality, with TWO deliberate exceptions: the type of `[]`
/// (`array<void>`, see `is_empty_array`) is equal to every `array<T>`, and the
/// type of `[:]` (`is_empty_map`) is equal to every `map<K, V>`. That is how an empty map
/// literal is accepted wherever a map of any type is expected (annotation,
/// argument, return, assignment, struct field) without threading an
/// "expected type" through the whole checker. `let m = [:]` with no
/// annotation is rejected separately (`EmptyMapNeedsType`); `let a = []`
/// with no annotation stays `array<void>`, as it always was.
impl PartialEq for Ty {
    fn eq(&self, other: &Ty) -> bool {
        match (self, other) {
            (Ty::Int, Ty::Int)
            | (Ty::Float, Ty::Float)
            | (Ty::Bool, Ty::Bool)
            | (Ty::String, Ty::String)
            | (Ty::Void, Ty::Void)
            | (Ty::Any, Ty::Any)
            | (Ty::AnyArray, Ty::AnyArray)
            | (Ty::AnyMap, Ty::AnyMap) => true,
            (Ty::Array(a), Ty::Array(b)) => self.is_empty_array() || other.is_empty_array() || a == b,
            (Ty::Named(a), Ty::Named(b)) => a == b,
            (Ty::Function { params: p1, ret: r1 }, Ty::Function { params: p2, ret: r2 }) => p1 == p2 && r1 == r2,
            (Ty::Optional(a), Ty::Optional(b)) => a == b,
            (Ty::NoneLit, Ty::NoneLit) => true,
            (Ty::Map(k1, v1), Ty::Map(k2, v2)) => {
                self.is_empty_map() || other.is_empty_map() || (k1 == k2 && v1 == v2)
            }
            _ => false,
        }
    }
}

/// A type as the USER writes it (`int`, `array<Ponto>`, `Ponto`), not
/// Rust's Debug shape (`Array(Named("Ponto"))`). Struct names go through
/// `display_name`, so a struct imported from another module reads as
/// `Ponto` rather than its internal mangled name.
impl std::fmt::Display for Ty {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Ty::Int => write!(f, "int"),
            Ty::Float => write!(f, "float"),
            Ty::Bool => write!(f, "bool"),
            Ty::String => write!(f, "string"),
            Ty::Void => write!(f, "void"),
            Ty::Array(inner) => write!(f, "array<{}>", inner),
            Ty::Map(..) if self.is_empty_map() => write!(f, "map<?, ?>"),
            Ty::Map(k, v) => write!(f, "map<{}, {}>", k, v),
            Ty::Named(n) => write!(f, "{}", spg_parser::ast::display_name(n)),
            // builtin-only types; never user-declarable, but a builtin
            // signature mismatch can still mention them
            Ty::Any => write!(f, "any"),
            Ty::AnyArray => write!(f, "array<any>"),
            Ty::AnyMap => write!(f, "map<any, any>"),
            Ty::Optional(inner) => write!(f, "{}?", inner),
            Ty::NoneLit => write!(f, "none"),
            Ty::Function { params, ret } => {
                let list: Vec<String> = params.iter().map(|p| p.to_string()).collect();
                write!(f, "function({}) -> {}", list.join(", "), ret)
            }
        }
    }
}

/// where a typed statement starts in its source file: the line a runtime
/// error (and its stack trace) points at
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SrcLoc {
    pub file: Option<String>,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypedBlock {
    pub stmts: Vec<TypedStmt>,
    /// one location per entry of `stmts`, same order
    pub locs: Vec<SrcLoc>,
    /// type of the block as a whole — only meaningful when the block is
    /// used as the then/else branch of an if-expression; Void otherwise
    pub ty: Ty,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypedExpr {
    IntLit(i64),
    FloatLit(f64),
    StringLit(String),
    BoolLit(bool),
    ArrayLit { items: Vec<TypedExpr>, ty: Ty },
    /// `["a": 1]` / `[:]`; an empty one has type `Map(Void, Void)`
    MapLit { entries: Vec<(TypedExpr, TypedExpr)>, ty: Ty },

    Ident { name: String, ty: Ty },

    /// the literal `none` (its type is `Ty::NoneLit`)
    NoneLit { ty: Ty },

    /// `left ?? right`: `left` (an optional) unless it is `none`, else `right`.
    /// `right` is only evaluated when `left` is `none`
    Coalesce { left: Box<TypedExpr>, right: Box<TypedExpr>, ty: Ty },

    /// `function(x: int) -> int { ... }`. `captures` are the enclosing function's
    /// local variables the body uses, by name; their values are copied when the
    /// lambda is created (see `Checker::resolve_name`). The body sees them as
    /// ordinary locals, ahead of the parameters.
    Lambda { params: Vec<Param>, return_type: Ty, body: TypedBlock, captures: Vec<(String, Ty)>, ty: Ty },

    /// a call through a function VALUE (a variable, a parameter, a call result).
    /// Named-function calls stay `Call`
    CallValue { callee: Box<TypedExpr>, args: Vec<TypedExpr>, ty: Ty },

    Binary {
        left: Box<TypedExpr>,
        op: BinaryOp,
        right: Box<TypedExpr>,
        ty: Ty,
    },

    Unary {
        op: UnaryOp,
        expr: Box<TypedExpr>,
        ty: Ty,
    },

    Call {
        callee: Box<TypedExpr>,
        args: Vec<TypedExpr>,
        ty: Ty, // return type of the called function
        /// `Some(id)` when `callee` resolved to a builtin — the checker
        /// already picked the right overload, so the compiler just reads
        /// this instead of re-matching the name+arity+types itself
        builtin: Option<BuiltinId>,
    },

    /// `arr[index]` — reading an element out of an array; `ty` is the
    /// array's element type, resolved by the checker so the compiler
    /// never has to re-derive it
    Index {
        array: Box<TypedExpr>,
        index: Box<TypedExpr>,
        ty: Ty,
    },

    /// `Name { field: value, ... }` — the checker has already validated
    /// every field is present, no unknown field was given, and each
    /// value's type matches its field's declared type. `fields` is
    /// reordered (if needed) to the struct's declaration order, so the
    /// compiler can emit field values positionally without consulting
    /// `struct_defs` itself.
    StructLit {
        name: String,
        fields: Vec<TypedExpr>,
        ty: Ty,
    },

    /// `obj.field` — reading a struct field; `ty` is that field's
    /// declared type
    Field {
        object: Box<TypedExpr>,
        field: String,
        ty: Ty,
    },

    IfExpr {
        cond: Box<TypedExpr>,
        then_branch: Box<TypedBlock>,
        else_branch: Box<TypedBlock>,
        ty: Ty, // shared type of both branches
    },

    /// `Color.Green` — variant number `index` of enum `name`. Compiles to a
    /// constant `Value::Enum`; `ty` is `Ty::Named(name)`.
    EnumLit { name: String, index: usize, ty: Ty },
    /// `c.value` on an enum value: the literal the variant was declared with
    /// (`ty` is that literal's type). `enum_name` finds the value table.
    EnumValue { operand: Box<TypedExpr>, enum_name: String, ty: Ty },

    /// an enum-typed expression read as the plain `int` the VM compares. Only
    /// the checker builds it (around the operands of `==` / `!=` on enums),
    /// so the existing `int` compare opcodes serve enums too. Emits nothing.
    AsInt(Box<TypedExpr>),
}

impl TypedExpr {
    pub fn ty(&self) -> &Ty {
        match self {
            TypedExpr::IntLit(_) => &Ty::Int,
            TypedExpr::FloatLit(_) => &Ty::Float,
            TypedExpr::StringLit(_) => &Ty::String,
            TypedExpr::BoolLit(_) => &Ty::Bool,
            TypedExpr::ArrayLit { ty, .. } => ty,
            TypedExpr::MapLit { ty, .. } => ty,
            TypedExpr::Ident { ty, .. } => ty,
            TypedExpr::Binary { ty, .. } => ty,
            TypedExpr::Unary { ty, .. } => ty,
            TypedExpr::Call { ty, .. } => ty,
            TypedExpr::Lambda { ty, .. } => ty,
            TypedExpr::NoneLit { ty } => ty,
            TypedExpr::Coalesce { ty, .. } => ty,
            TypedExpr::CallValue { ty, .. } => ty,
            TypedExpr::Index { ty, .. } => ty,
            TypedExpr::StructLit { ty, .. } => ty,
            TypedExpr::Field { ty, .. } => ty,
            TypedExpr::IfExpr { ty, .. } => ty,
            TypedExpr::EnumLit { ty, .. } => ty,
            TypedExpr::EnumValue { ty, .. } => ty,
            TypedExpr::AsInt(_) => &Ty::Int,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypedStmt {
    Let { name: String, ty: Ty, value: TypedExpr, exported: bool },
    Const { name: String, ty: Ty, value: TypedExpr, exported: bool },

    /// reassignment to an existing `let` — the checker has already
    /// confirmed the target exists, isn't const, and the value's type
    /// matches, so the compiler can treat this exactly like Let's
    /// SetLocal emission with no extra validation
    Assign { name: String, value: TypedExpr },

    /// `arr[index] = value` — the checker has already confirmed `array`
    /// is an array, `index` is Int, and `value`'s type matches the
    /// array's element type
    IndexAssign { array: TypedExpr, index: TypedExpr, value: TypedExpr },

    /// `arr[index] OP= value` — the checker has already confirmed
    /// `elem_type OP value_type` typechecks and produces back
    /// `elem_type`. Kept separate from IndexAssign (rather than
    /// desugared into it) so the compiler evaluates `array`/`index`
    /// exactly once — see `Stmt::IndexCompoundAssign`'s doc comment in
    /// spg-parser for why that matters.
    IndexCompoundAssign { array: TypedExpr, index: TypedExpr, op: BinaryOp, value: TypedExpr },

    /// `obj.field = value` — the checker has already confirmed `object`
    /// is a struct declaring `field`, and `value`'s type matches that
    /// field's declared type
    FieldAssign { object: TypedExpr, field: String, value: TypedExpr },

    /// `obj.field OP= value` — same relationship to FieldAssign that
    /// IndexCompoundAssign has to IndexAssign
    FieldCompoundAssign { object: TypedExpr, field: String, op: BinaryOp, value: TypedExpr },

    IfStmt {
        cond: TypedExpr,
        then_branch: TypedBlock,
        else_branch: Option<TypedBlock>,
    },

    While { cond: TypedExpr, body: TypedBlock },

    ForIn { var_name: String, elem_ty: Ty, iterable: TypedExpr, body: TypedBlock },

    /// `for i in start..end [step s]` (end exclusive)
    ForRange {
        var_name: String,
        start: TypedExpr,
        end: TypedExpr,
        step: RangeStep,
        /// true when the body contains `var_name = ...` (resolved to this very
        /// variable): the compiler must then give the body its own COPY of the
        /// counter. When false the counter itself is the variable (faster).
        var_assigned: bool,
        body: TypedBlock,
    },

    Return { value: Option<TypedExpr> },

    Break,
    Continue,

    /// `throw value`: the value is always a string (`ThrowNotString` otherwise)
    Throw { value: TypedExpr },
    Try {
        try_block: TypedBlock,
        catch_var: String,
        catch_ty: Ty,
        catch_block: TypedBlock,
    },

    FunctionDecl {
        name: String,
        params: Vec<Param>, // params keep their original (always-explicit) annotation
        return_type: Ty,
        body: TypedBlock,
        exported: bool,
    },

    StructDecl { name: String, fields: Vec<Param>, exported: bool },

    /// `variants` in declaration order: a variant's position is its identity
    EnumDecl { name: String, variants: Vec<String>, values: Vec<spg_parser::ast::EnumValue>, exported: bool },

    Import { names: Vec<String>, from: String },

    ExprStmt(TypedExpr),
}

/// the step of a `for .. in a..b step s`, classified once by the
/// typechecker so the compiler can pick the cheapest loop shape
#[derive(Debug, Clone, PartialEq)]
pub enum RangeStep {
    /// no `step`: +1
    One,
    /// a non-zero integer literal (the sign is known at compile time)
    Const(i64),
    /// any other int expression (sign and zero checked at run time)
    Dynamic(Box<TypedExpr>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypedProgram {
    pub stmts: Vec<TypedStmt>,
    /// one location per entry of `stmts`, same order
    pub locs: Vec<SrcLoc>,
}
