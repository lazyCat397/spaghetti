//! Resolved types and the typed AST the typechecker spits out.
//!
//! `Ty` is the checker's internal, comparable type. not the same as
//! `TypeExpr` (spg_parser::ast), which is only the syntax the
//! user typed. every expression in `TypedProgram` carries a `Ty`
//! the checker resolved, so `spg-compiler` never has to guess types again.

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
    /// `map<K, V>`. `Map(Void, Void)` is the type of `[:]` (see `is_empty_map`). empty on purpose.
    Map(Box<Ty>, Box<Ty>),
    Named(String), // struct
    /// accepts any type as a parameter. reserved for
    /// builtins (e.g. print). users can't declare it, so it
    /// doesn't turn into sneaky dynamic typing everywhere. nope.
    Any,
    /// accepts `array<T>` for any element type T, as a builtin
    /// parameter. lets array builtins (len, push, reverse, ...) work on
    /// any element type without becoming user-facing
    /// generics. users can't declare it. same vibe as `Any`.
    AnyArray,
    /// accepts `map<K, V>` for any K and V, as a builtin parameter
    /// (`@map.*`). users can't declare it. same spirit as `AnyArray`.
    AnyMap,
    /// `function(P1, P2) -> R`: the type of a function value (a named function,
    /// a lambda, or a variable holding one of those)
    Function { params: Vec<Ty>, ret: Box<Ty> },
    /// `T?`: a value of type T, or `none`. the question mark is doing a lot of work.
    Optional(Box<Ty>),
    /// the type of the literal `none` on its own. fits every optional type. easygoing.
    NoneLit,
    /// `task<T>`: the handle a `-> task<T>` call gives back. `.wait()` gets the T out.
    Task(Box<Ty>),
}

impl Ty {
    /// can a value of type `found` go where `self` is expected?
    /// the same type, or (for `T?`) a `T`, a `T?`, or `none`. never the other
    /// way round: a `T?` does NOT fit a `T`. obviously.
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

    /// the type of `[:]`: a map with no known key/value types. it is
    /// `Map(Void, Void)` (users can't write `map<void, void>`, nice try).
    pub fn is_empty_array(&self) -> bool {
        matches!(self, Ty::Array(inner) if matches!(**inner, Ty::Void))
    }

    pub fn is_empty_map(&self) -> bool {
        matches!(self, Ty::Map(k, v) if matches!(**k, Ty::Void) && matches!(**v, Ty::Void))
    }
}

/// structural equality, with TWO deliberate exceptions: the type of `[]`
/// (`array<void>`, see `is_empty_array`) equals every `array<T>`, and the
/// type of `[:]` (`is_empty_map`) equals every `map<K, V>`. that's how an empty map
/// literal gets accepted wherever a map is expected (annotation,
/// argument, return, assignment, struct field) without dragging an
/// "expected type" through the whole checker. `let m = [:]` with no
/// annotation is rejected (`EmptyMapNeedsType`). `let a = []`
/// with no annotation stays `array<void>`, like it always did.
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
            (Ty::Task(a), Ty::Task(b)) => a == b,
            (Ty::NoneLit, Ty::NoneLit) => true,
            (Ty::Map(k1, v1), Ty::Map(k2, v2)) => {
                self.is_empty_map() || other.is_empty_map() || (k1 == k2 && v1 == v2)
            }
            _ => false,
        }
    }
}

/// a type as the USER writes it (`int`, `array<Point>`, `Point`), not
/// Rust's Debug mess (`Array(Named("Point"))`). struct names go through
/// `display_name`, so a struct imported from another module reads as
/// `Point`, not its mangled internal name. tidy.
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
            // builtin-only types. never user-declarable, but a builtin
            // signature mismatch can still mention them. sneaky.
            Ty::Any => write!(f, "any"),
            Ty::AnyArray => write!(f, "array<any>"),
            Ty::AnyMap => write!(f, "map<any, any>"),
            Ty::Optional(inner) => write!(f, "{}?", inner),
            Ty::Task(inner) => write!(f, "task<{}>", inner),
            Ty::NoneLit => write!(f, "none"),
            Ty::Function { params, ret } => {
                let list: Vec<String> = params.iter().map(|p| p.to_string()).collect();
                write!(f, "function({}) -> {}", list.join(", "), ret)
            }
        }
    }
}

/// where a typed statement starts in its source file. the line a runtime
/// error (and its stack trace) points at. the blame line.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SrcLoc {
    pub file: Option<String>,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypedBlock {
    pub stmts: Vec<TypedStmt>,
    /// one location per entry of `stmts`, same order. again. don't shuffle.
    pub locs: Vec<SrcLoc>,
    /// type of the block as a whole. only means something when the block is
    /// the then/else branch of an if-expression. void otherwise.
    pub ty: Ty,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypedExpr {
    IntLit(i64),
    FloatLit(f64),
    StringLit(String),
    BoolLit(bool),
    ArrayLit { items: Vec<TypedExpr>, ty: Ty },
    /// `["a": 1]` / `[:]`. an empty one has type `Map(Void, Void)`
    MapLit { entries: Vec<(TypedExpr, TypedExpr)>, ty: Ty },

    Ident { name: String, ty: Ty },

    /// the literal `none` (its type is `Ty::NoneLit`). lonely.
    NoneLit { ty: Ty },

    /// `left ?? right`: `left` (an optional) unless it's `none`, else `right`.
    /// `right` only runs when `left` is `none`. lazy, on purpose.
    Coalesce { left: Box<TypedExpr>, right: Box<TypedExpr>, ty: Ty },

    /// `function(x: int) -> int { ... }`. `captures` are the enclosing function's
    /// locals the body uses, by name. values get copied when the
    /// lambda is made (see `Checker::resolve_name`). the body sees them as
    /// regular locals, before the parameters.
    Lambda { params: Vec<Param>, return_type: Ty, body: TypedBlock, captures: Vec<(String, Ty)>, ty: Ty },

    /// a call through a function VALUE (a variable, a parameter, a call result).
    /// named-function calls stay `Call`. no change there.
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
        /// `Some(id)` when `callee` resolved to a builtin. the checker
        /// already picked the overload, so the compiler just reads
        /// this instead of re-matching name+arity+types itself. lazy compiler.
        builtin: Option<BuiltinId>,
    },

    /// `arr[index]`: reading an element. `ty` is the
    /// array's element type, resolved by the checker so the compiler
    /// never has to figure it out again. (it's the same answer, promise.)
    Index {
        array: Box<TypedExpr>,
        index: Box<TypedExpr>,
        ty: Ty,
    },

    /// `Name { field: value, ... }`. the checker already validated
    /// every field is there, no unknown field sneaked in, and each
    /// value's type matches its field. `fields` gets
    /// reordered (if needed) to the declaration order, so the
    /// compiler can emit them by position without looking at
    /// `struct_defs`. fewer lookups, happier compiler.
    StructLit {
        name: String,
        fields: Vec<TypedExpr>,
        ty: Ty,
    },

    /// `obj.field`: reading a struct field. `ty` is that field's
    /// declared type. shocking.
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

    /// `Color.Green`: variant number `index` of enum `name`. compiles to a
    /// constant `Value::Enum`. `ty` is `Ty::Named(name)`.
    EnumLit { name: String, index: usize, ty: Ty },
    /// `c.value` on an enum: the literal the variant was declared with
    /// (`ty` is that literal's type). `enum_name` finds the value table.
    EnumValue { operand: Box<TypedExpr>, enum_name: String, ty: Ty },

    /// an enum expression read as a plain `int`, which is what the VM compares. only
    /// the checker builds it (around the operands of `==` / `!=` on enums),
    /// so the existing int compare opcodes handle enums too. emits nothing. free.
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

    /// reassignment to an existing `let`. the checker already
    /// confirmed the target exists, isn't const, and the type matches,
    /// so the compiler treats it exactly like a Let's
    /// SetLocal, no extra checks. trust the checker.
    Assign { name: String, value: TypedExpr },

    /// `arr[index] = value`. the checker already confirmed `array`
    /// is an array, `index` is Int, and `value`'s type matches the
    /// array's element type. yes, again.
    IndexAssign { array: TypedExpr, index: TypedExpr, value: TypedExpr },

    /// `arr[index] OP= value`. the checker already confirmed
    /// `elem_type OP value_type` typechecks and gives back
    /// `elem_type`. kept separate from IndexAssign (not
    /// desugared into it) so the compiler evaluates `array`/`index`
    /// exactly once. see `Stmt::IndexCompoundAssign`'s doc in
    /// spg-parser for why this matters. (it matters a lot.)
    IndexCompoundAssign { array: TypedExpr, index: TypedExpr, op: BinaryOp, value: TypedExpr },

    /// `obj.field = value`. the checker already confirmed `object`
    /// is a struct with `field`, and `value`'s type matches that
    /// field's declared type. same as always.
    FieldAssign { object: TypedExpr, field: String, value: TypedExpr },

    /// `obj.field OP= value`. same deal with FieldAssign as
    /// IndexCompoundAssign has with IndexAssign
    FieldCompoundAssign { object: TypedExpr, field: String, op: BinaryOp, value: TypedExpr },

    IfStmt {
        cond: TypedExpr,
        then_branch: TypedBlock,
        else_branch: Option<TypedBlock>,
    },

    /// a `{ }` scope of its own. the checker wraps a lowered `when` match in one
    /// so its hidden subject local dies at the end. good riddance.
    Block { body: TypedBlock },

    While { cond: TypedExpr, body: TypedBlock },

    ForIn { var_name: String, elem_ty: Ty, iterable: TypedExpr, body: TypedBlock },

    /// `for i in start..end [step s]` (end exclusive, always)
    ForRange {
        var_name: String,
        start: TypedExpr,
        end: TypedExpr,
        step: RangeStep,
        /// true when the body contains `var_name = ...` (resolved to this very
        /// variable). then the compiler gives the body its own COPY of the
        /// counter. when false, the counter IS the variable. faster. sneaky.
        var_assigned: bool,
        body: TypedBlock,
    },

    Return { value: Option<TypedExpr> },

    Break,
    Continue,

    /// `throw value`: always a string (`ThrowNotString` otherwise). no numbers, sorry.
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

    /// `variants` in declaration order. a variant's position is its identity
    EnumDecl { name: String, variants: Vec<String>, values: Vec<spg_parser::ast::EnumValue>, exported: bool },

    Import { names: Vec<String>, from: String },

    ExprStmt(TypedExpr),
}

/// the step of a `for .. in a..b step s`, classified once by the
/// typechecker so the compiler picks the cheapest loop shape
#[derive(Debug, Clone, PartialEq)]
pub enum RangeStep {
    /// no `step`: +1. the default. boring, fast.
    One,
    /// a non-zero integer literal (sign known at compile time)
    Const(i64),
    /// any other int expression (sign and zero checked at runtime)
    Dynamic(Box<TypedExpr>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypedProgram {
    pub stmts: Vec<TypedStmt>,
    /// one location per entry of `stmts`, same order. again. don't shuffle.
    pub locs: Vec<SrcLoc>,
}
