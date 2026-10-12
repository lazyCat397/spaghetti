//! Fatal type errors. The checker collects ALL of these in one pass
//! instead of stopping at the first one — matches the language's rigid,
//! no-silent-fallback philosophy, but still gives the user everything
//! wrong in one shot instead of a fix-rerun-fix loop.

use spg_parser::ast::{display_name, Pos};
use crate::typed_ast::Ty;
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum TypeError {
    /// `let x: int = "text"` — annotation and value disagree
    AnnotationMismatch { expected: Ty, found: Ty, pos: Pos },

    /// function declared `-> int` but a `return` gives back something else
    ReturnTypeMismatch { expected: Ty, found: Ty, pos: Pos },

    /// a code path in a non-void function has no `return` at all
    MissingReturn { function_name: String, expected: Ty, pos: Pos },

    /// the two branches of an if-EXPRESSION disagree in type
    IfExprBranchMismatch { then_ty: Ty, else_ty: Ty, pos: Pos },

    /// `when (x)` where `x` is not an int, a string or an enum
    MatchSubjectType { found: Ty, pos: Pos },
    /// a case value whose type differs from the matched value's
    MatchCaseType { expected: Ty, found: Ty, pos: Pos },
    /// an enum match without `else` that leaves some variants out
    MatchNonExhaustive { missing: Vec<String>, pos: Pos },

    /// identifier used but never declared in any visible scope
    UndefinedVariable { name: String, pos: Pos },

    /// callee is not a function (or not defined at all)
    UndefinedFunction { name: String, pos: Pos },

    /// a pre-namespace builtin name (`sqrt`, `toUpper`, `clock`, ...) that
    /// now lives under a namespace; `replacement` is the new qualified name
    RemovedBuiltin { name: String, replacement: String, pos: Pos },

    /// `math.sqrt(...)`, `list.push(...)`, `transform.toInt(...)`, `time.run()`:
    /// the namespace spelling used before namespaces got their `@`.
    /// `replacement` is the current name when one exists.
    OldNamespace { name: String, replacement: Option<String>, pos: Pos },

    /// a call whose callee is not a plain function name (`p.x()`, `f()()`,
    /// `(a + b)()`): only named functions and builtins can be called
    NotCallable { pos: Pos },

    /// a lambda assigns a variable of the function it was written in: captured values are copies
    CaptureAssign { name: String, pos: Pos },

    /// wrong number of arguments in a call
    ArityMismatch { function_name: String, expected: usize, found: usize, pos: Pos },

    /// argument type doesn't match the parameter's declared type
    ArgTypeMismatch { function_name: String, param_index: usize, expected: Ty, found: Ty, pos: Pos },

    /// operator used with types it doesn't support (e.g. "a" - 1)
    InvalidOperandTypes { op: String, left: Ty, right: Ty, pos: Pos },

    /// `for x in y` where `y` isn't an array
    NotIterable { found: Ty, pos: Pos },

    /// `for i in a..b step s` where `a`, `b` or `s` isn't an int
    RangeNotInt { which: &'static str, found: Ty, pos: Pos },

    /// `for i in a..b step 0`: a literal zero step can never finish
    RangeZeroStep { pos: Pos },

    /// re-declaring a name already bound in the same scope
    DuplicateDeclaration { name: String, pos: Pos },
    /// `#> strict`: a name that hides another one from an outer scope
    ShadowedDeclaration { name: String, pos: Pos },
    /// `#> strict`: code after `return` / `break` / `continue` / `throw`
    UnreachableCode { keyword: &'static str, pos: Pos },

    /// an `if`/`while` condition isn't Bool
    NonBoolCondition { found: Ty, pos: Pos },

    /// `x = valor` where `x` was declared with `const`
    AssignToConst { name: String, pos: Pos },

    /// `x = valor` where the value's type disagrees with `x`'s declared type
    AssignTypeMismatch { name: String, expected: Ty, found: Ty, pos: Pos },

    /// `break`/`continue` used outside of any enclosing while/for-in loop
    LoopControlOutsideLoop { keyword: String, pos: Pos },

    /// no trailing `-> Type` was given and the function's `return`
    /// statements disagree on type, so nothing could be inferred —
    /// unlike ReturnTypeMismatch (which compares against a type the
    /// user DID declare), this fires when there was nothing to compare
    /// against in the first place
    CannotInferReturnType { function_name: String, found_types: Vec<Ty>, pos: Pos },

    /// `expr[index]` or `expr[index] = ...` where `expr` isn't an array
    NotIndexable { found: Ty, pos: Pos },

    /// an array index expression isn't `int` (e.g. `arr["x"]`)
    IndexTypeMismatch { found: Ty, pos: Pos },

    /// a map key type that cannot be hashed: only int, string, bool and enums
    MapKeyType { found: Ty, pos: Pos },

    /// `m[k]` / `@map.has(m, k)` / a literal entry whose key is not the map's key type
    MapKeyMismatch { expected: Ty, found: Ty, pos: Pos },

    /// a value that is not the map's value type (`m[k] = v`, literal entry, `@map.get` fallback)
    MapValueMismatch { expected: Ty, found: Ty, pos: Pos },

    /// `[1, 2.5]`: every item of an array literal has the first item's type
    ArrayItemMismatch { expected: Ty, found: Ty, pos: Pos },

    /// `@array.fill(n, x)` with an `x` that is not int, float, bool or string
    ArrayFillType { found: Ty, pos: Pos },

    /// `let m = [:]` with nothing to say what the map holds
    EmptyMapNeedsType { pos: Pos },

    /// `let x = none`: nothing says which optional type this is
    NoneNeedsType { pos: Pos },

    /// `a ?? b` where `a` is not an optional
    CoalesceNeedsOptional { found: Ty, pos: Pos },

    /// `let x = <expr>` / `const` where the value has type void (e.g. an `if`
    /// whose branches only call print): there is no value to store
    VoidValue { pos: Pos },

    /// `arr[i] = value` where `value`'s type disagrees with the array's
    /// element type
    IndexAssignTypeMismatch { expected: Ty, found: Ty, pos: Pos },

    /// `Name { .. }` where `Name` was never declared with `struct`
    UndefinedStruct { name: String, pos: Pos },

    /// a struct literal is missing one or more of its declared fields
    MissingFields { struct_name: String, missing: Vec<String>, pos: Pos },

    /// a struct literal names a field the struct doesn't declare
    UnknownField { struct_name: String, field: String, pos: Pos },

    /// a struct literal field's value doesn't match that field's
    /// declared type
    StructFieldTypeMismatch { struct_name: String, field: String, expected: Ty, found: Ty, pos: Pos },

    /// `expr.field` where `expr` isn't a struct at all
    NotAStruct { found: Ty, pos: Pos },

    /// `Color.Purple` where enum `Color` has no such variant
    NoSuchVariant { enum_name: String, variant: String, variants: Vec<String>, pos: Pos },

    /// the bare name of an enum used as a value (`let x = Color`)
    EnumAsValue { name: String, pos: Pos },

    /// `expr.field` where `expr` IS a struct but doesn't declare `field`
    NoSuchField { struct_name: String, field: String, pos: Pos },

    /// `p.f()` where struct `P` has no method `f`. `is_field` is true when
    /// `f` IS a field of `P` (so the hint can say fields are not callable)
    NoSuchMethod { struct_name: String, method: String, is_field: bool, pos: Pos },

    /// `x.f()` where `x` is not a struct at all (an int, a string, an array...)
    NoMethodsOnType { found: Ty, method: String, pos: Pos },

    /// `throw 5`: exceptions carry a string
    ThrowNotString { found: Ty, pos: Pos },
    /// `catch (e: Point)`: what `catch` receives is the thrown string
    CatchNotString { found: Ty, pos: Pos },
    /// `s.nope()` / `xs.nope()` / `m.nope()`: no builtin of `@str`/`@array`/`@map` by that
    /// name takes the value first
    NoSuchNativeMethod { found: Ty, method: String, pos: Pos },

    /// `p.f` (no call) where `f` is a METHOD of the struct, not a field
    MethodNotCalled { struct_name: String, method: String, pos: Pos },
}

impl TypeError {
    /// the `Pos` carried by whichever variant this is — one place that
    /// knows every variant has exactly one `pos` field, so `Display`
    /// below doesn't need its own copy of this same 25-arm match just
    /// to find the file/line/col to print.
    pub fn pos(&self) -> &Pos {
        match self {
            TypeError::AnnotationMismatch { pos, .. }
            | TypeError::ReturnTypeMismatch { pos, .. }
            | TypeError::MissingReturn { pos, .. }
            | TypeError::IfExprBranchMismatch { pos, .. }
            | TypeError::MatchSubjectType { pos, .. }
            | TypeError::MatchCaseType { pos, .. }
            | TypeError::MatchNonExhaustive { pos, .. }
            | TypeError::UndefinedVariable { pos, .. }
            | TypeError::UndefinedFunction { pos, .. }
            | TypeError::RemovedBuiltin { pos, .. }
            | TypeError::OldNamespace { pos, .. }
            | TypeError::NotCallable { pos, .. }
            | TypeError::CaptureAssign { pos, .. }
            | TypeError::ArityMismatch { pos, .. }
            | TypeError::ArgTypeMismatch { pos, .. }
            | TypeError::InvalidOperandTypes { pos, .. }
            | TypeError::NotIterable { pos, .. }
            | TypeError::RangeNotInt { pos, .. }
            | TypeError::RangeZeroStep { pos, .. }
            | TypeError::DuplicateDeclaration { pos, .. }
            | TypeError::ShadowedDeclaration { pos, .. }
            | TypeError::UnreachableCode { pos, .. }
            | TypeError::NonBoolCondition { pos, .. }
            | TypeError::AssignToConst { pos, .. }
            | TypeError::AssignTypeMismatch { pos, .. }
            | TypeError::LoopControlOutsideLoop { pos, .. }
            | TypeError::CannotInferReturnType { pos, .. }
            | TypeError::NotIndexable { pos, .. }
            | TypeError::IndexTypeMismatch { pos, .. }
            | TypeError::MapKeyType { pos, .. }
            | TypeError::MapKeyMismatch { pos, .. }
            | TypeError::MapValueMismatch { pos, .. }
            | TypeError::EmptyMapNeedsType { pos, .. }
            | TypeError::NoneNeedsType { pos, .. }
            | TypeError::CoalesceNeedsOptional { pos, .. }
            | TypeError::VoidValue { pos, .. }
            | TypeError::ArrayFillType { pos, .. }
            | TypeError::ArrayItemMismatch { pos, .. }
            | TypeError::IndexAssignTypeMismatch { pos, .. }
            | TypeError::UndefinedStruct { pos, .. }
            | TypeError::MissingFields { pos, .. }
            | TypeError::UnknownField { pos, .. }
            | TypeError::StructFieldTypeMismatch { pos, .. }
            | TypeError::NotAStruct { pos, .. }
            | TypeError::NoSuchVariant { pos, .. }
            | TypeError::EnumAsValue { pos, .. }
            | TypeError::NoSuchField { pos, .. }
            | TypeError::NoSuchMethod { pos, .. }
            | TypeError::NoMethodsOnType { pos, .. }
            | TypeError::MethodNotCalled { pos, .. }
            | TypeError::NoSuchNativeMethod { pos, .. }
            | TypeError::ThrowNotString { pos, .. }
            | TypeError::CatchNotString { pos, .. } => pos,
        }
    }

    /// the full `"type error ...: "` prefix every message below starts
    /// with. spg-modules stamps a `Pos.file` on every statement of every
    /// program it resolves — single-file ones included — so in
    /// practice `file` is always `Some` here and every error names its
    /// file, not just its line/column. The `None` arm only exists for
    /// `Pos` values built directly by something other than spg-modules
    /// (a hand-built `Program` in a test, say); it falls back to the
    /// old line/column-only wording rather than printing a placeholder.
    /// the message alone, without the `type error in <file> at line X,
    /// column Y: ` prefix — for consumers (the language server) that carry
    /// the position separately
    pub fn message(&self) -> String {
        let full = self.to_string();
        match full.strip_prefix(&self.location()) {
            Some(rest) => rest.to_string(),
            None => full,
        }
    }

    fn location(&self) -> String {
        let pos = self.pos();
        match &pos.file {
            Some(file) => format!("type error in {} at line {}, column {}: ", file, pos.line, pos.col),
            None => format!("type error at line {}, column {}: ", pos.line, pos.col),
        }
    }
}

impl fmt::Display for TypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.fmt_base(f)?;
        // an optional where a plain value was needed: say how to get out of it
        let optional = |t: &Ty| matches!(t, Ty::Optional(_));
        let involved = match self {
            TypeError::InvalidOperandTypes { left, right, .. } => optional(left) || optional(right),
            TypeError::AnnotationMismatch { found, .. }
            | TypeError::ReturnTypeMismatch { found, .. }
            | TypeError::ArgTypeMismatch { found, .. }
            | TypeError::AssignTypeMismatch { found, .. }
            | TypeError::IndexAssignTypeMismatch { found, .. }
            | TypeError::NotAStruct { found, .. } => optional(found),
            _ => false,
        };
        if involved {
            write!(f, " (a T? may be none: check it with `!= none` first, or give it a default with `??`)")?;
        }
        Ok(())
    }
}

impl TypeError {
    fn fmt_base(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TypeError::AnnotationMismatch { expected, found, .. } => write!(
                f, "{}declared type is {} but value is {}",
                self.location(), expected, found
            ),
            TypeError::ReturnTypeMismatch { expected, found, .. } => write!(
                f, "{}function returns {} but this return gives {}",
                self.location(), expected, found
            ),
            TypeError::MissingReturn { function_name, expected, .. } => write!(
                f, "{}function '{}' must return {} on every path, but a return is missing",
                self.location(), display_name(function_name), expected
            ),
            TypeError::MatchSubjectType { found, .. } => write!(
                f, "{}'when' can only match an int, a string or an enum, but this value is {}",
                self.location(), found
            ),
            TypeError::MatchCaseType { expected, found, .. } => write!(
                f, "{}this case is {}, but the matched value is {}",
                self.location(), found, expected
            ),
            TypeError::MatchNonExhaustive { missing, .. } => write!(
                f, "{}'when' does not cover every variant of the enum (missing: {}); add those cases or an 'else'",
                self.location(), missing.join(", ")
            ),
            TypeError::IfExprBranchMismatch { then_ty, else_ty, .. } => write!(
                f, "{}if-expression branches disagree in type (then: {}, else: {})",
                self.location(), then_ty, else_ty
            ),
            TypeError::UndefinedVariable { name, .. } => write!(
                f, "{}undefined variable '{}'",
                self.location(), display_name(name)
            ),
            TypeError::OldNamespace { name, replacement, .. } => match replacement {
                Some(r) => write!(
                    f, "{}'{}' uses the old namespace spelling; namespaces are written with '@' now, use '{}'",
                    self.location(), name, r
                ),
                None => write!(
                    f, "{}'{}' is not a builtin; namespaces are written with '@' (@str, @array, @map, @into, @math, @time, @file, @sys)",
                    self.location(), name
                ),
            },
            TypeError::NotCallable { .. } => write!(
                f, "{}only functions can be called, and this value is not a function",
                self.location()
            ),
            TypeError::CaptureAssign { name, .. } => write!(
                f, "{}a lambda cannot assign '{}': it is a copy of a variable from the function it was written in",
                self.location(), display_name(name)
            ),
            TypeError::UndefinedFunction { name, .. } => match name.split_once('.') {
                // `@math.foo`: the namespace exists, the member doesn't
                Some((ns, member)) => write!(
                    f, "{}namespace '{}' has no member '{}'",
                    self.location(), ns, member
                ),
                None => write!(
                    f, "{}undefined function '{}'",
                    self.location(), display_name(name)
                ),
            },
            TypeError::RemovedBuiltin { name, replacement, .. } => write!(
                f, "{}'{}' is no longer a global function; use '{}'",
                self.location(), display_name(name), replacement
            ),
            TypeError::ArityMismatch { function_name, expected, found, .. } => write!(
                f, "{}'{}' expects {} argument(s), got {}",
                self.location(), display_name(function_name), expected, found
            ),
            TypeError::ArgTypeMismatch { function_name, param_index, expected, found, .. } => write!(
                f, "{}'{}' argument #{} expects {}, got {}",
                self.location(), display_name(function_name), param_index + 1, expected, found
            ),
            TypeError::InvalidOperandTypes { op, left, right, .. } => write!(
                f, "{}operator {} not supported for {} and {}",
                self.location(), op, left, right
            ),
            TypeError::RangeNotInt { which, found, .. } => write!(
                f, "{}the {} of a range must be int, found {}",
                self.location(), which, found
            ),
            TypeError::RangeZeroStep { .. } => write!(
                f, "{}a range step cannot be zero",
                self.location()
            ),
            TypeError::NotIterable { found, .. } => write!(
                f, "{}type {} is not iterable",
                self.location(), found
            ),
            TypeError::DuplicateDeclaration { name, .. } => write!(
                f, "{}'{}' is already declared in this scope",
                self.location(), display_name(name)
            ),
            TypeError::ShadowedDeclaration { name, .. } => write!(
                f, "{}'{}' hides a name from an outer scope (not allowed with #> strict)",
                self.location(), display_name(name)
            ),
            TypeError::UnreachableCode { keyword, .. } => write!(
                f, "{}code after '{}' can never run (not allowed with #> strict)",
                self.location(), keyword
            ),
            TypeError::NonBoolCondition { found, .. } => write!(
                f, "{}condition must be bool, found {}",
                self.location(), found
            ),
            TypeError::AssignToConst { name, .. } => write!(
                f, "{}cannot assign to '{}' — it's declared as const",
                self.location(), display_name(name)
            ),
            TypeError::AssignTypeMismatch { name, expected, found, .. } => write!(
                f, "{}cannot assign {} to '{}' — it's declared as {}",
                self.location(), found, display_name(name), expected
            ),
            TypeError::LoopControlOutsideLoop { keyword, .. } => write!(
                f, "{}'{}' used outside of a loop",
                self.location(), keyword
            ),
            TypeError::CannotInferReturnType { function_name, found_types, .. } => write!(
                f, "{}cannot infer return type of '{}' — its return statements disagree ({}); add an explicit '-> Type'",
                self.location(), display_name(function_name),
                found_types.iter().map(|t| t.to_string()).collect::<Vec<_>>().join(", ")
            ),
            TypeError::NotIndexable { found, .. } => write!(
                f, "{}type {} cannot be indexed with '[]'",
                self.location(), found
            ),
            TypeError::MapKeyType { found, .. } => write!(
                f, "{}map keys must be int, string, bool or an enum, found {}",
                self.location(), found
            ),
            TypeError::MapKeyMismatch { expected, found, .. } => write!(
                f, "{}this map's keys are {}, found {}",
                self.location(), expected, found
            ),
            TypeError::MapValueMismatch { expected, found, .. } => write!(
                f, "{}this map's values are {}, found {}",
                self.location(), expected, found
            ),
            TypeError::ArrayItemMismatch { expected, found, .. } => write!(
                f, "{}all items of an array must have the same type: expected {}, found {}",
                self.location(), expected, found
            ),
            TypeError::ArrayFillType { found, .. } => write!(
                f, "{}@array.fill repeats ONE value, so it only takes int, float, bool or string (an array or struct would be shared, not copied), found {}",
                self.location(), found
            ),
            TypeError::VoidValue { .. } => write!(
                f, "{}this expression has no value (void), so it cannot be stored in a variable",
                self.location()
            ),
            TypeError::NoneNeedsType { .. } => write!(
                f, "{}cannot tell which type 'none' belongs to; write it, e.g. 'let x: int? = none'",
                self.location()
            ),
            TypeError::CoalesceNeedsOptional { found, .. } => write!(
                f, "{}'??' needs an optional value on its left (like 'int?'), but found {}",
                self.location(), found
            ),
            TypeError::EmptyMapNeedsType { .. } => write!(
                f, "{}cannot tell what '[:]' holds; write the type, e.g. 'let m: map<string, int> = [:]'",
                self.location()
            ),
            TypeError::IndexTypeMismatch { found, .. } => write!(
                f, "{}array index must be int, found {}",
                self.location(), found
            ),
            TypeError::IndexAssignTypeMismatch { expected, found, .. } => write!(
                f, "{}cannot assign {} into an array of {}",
                self.location(), found, expected
            ),
            TypeError::UndefinedStruct { name, .. } => write!(
                f, "{}undefined struct '{}'",
                self.location(), display_name(name)
            ),
            TypeError::MissingFields { struct_name, missing, .. } => write!(
                f, "{}struct literal for '{}' is missing field(s): {}",
                self.location(), display_name(struct_name), missing.join(", ")
            ),
            TypeError::UnknownField { struct_name, field, .. } => write!(
                f, "{}struct '{}' has no field '{}'",
                self.location(), display_name(struct_name), field
            ),
            TypeError::StructFieldTypeMismatch { struct_name, field, expected, found, .. } => write!(
                f, "{}field '{}' of struct '{}' expects {}, found {}",
                self.location(), field, display_name(struct_name), expected, found
            ),
            TypeError::NoSuchVariant { enum_name, variant, variants, .. } => write!(
                f, "{}enum '{}' has no variant '{}' (variants: {})",
                self.location(), display_name(enum_name), variant, variants.join(", ")
            ),
            TypeError::EnumAsValue { name, .. } => write!(
                f, "{}'{}' is an enum, not a value; pick a variant ('{}.<Variant>') or loop over it ('for x in {}')",
                self.location(), display_name(name), display_name(name), display_name(name)
            ),
            TypeError::NotAStruct { found, .. } => write!(
                f, "{}type {} has no fields (not a struct)",
                self.location(), found
            ),
            TypeError::NoMethodsOnType { found, method, .. } => write!(
                f, "{}type {} has no methods (cannot call '.{}()' on it)",
                self.location(), found, method
            ),
            TypeError::ThrowNotString { found, .. } => write!(
                f, "{}'throw' takes a string (the message), found {}; build one with @into.string(...) or '+'",
                self.location(), found
            ),
            TypeError::CatchNotString { found, .. } => write!(
                f, "{}'catch' receives the thrown string, so its variable is a string; write 'catch (e)' or 'catch (e: string)', not {}",
                self.location(), found
            ),
            TypeError::NoSuchNativeMethod { found, method, .. } => match found {
                Ty::Int | Ty::Float => write!(
                    f, "{}type {} has no method '{}': its methods are the @math and @into functions that take it first (x.abs() is @math.abs(x))",
                    self.location(), found, method
                ),
                _ => {
                    let ns = match found {
                        Ty::String => "@str",
                        Ty::Array(_) => "@array",
                        _ => "@map",
                    };
                    write!(
                        f, "{}type {} has no method '{}': its methods are the {} functions that take it first (xs.len() is {}.len(xs))",
                        self.location(), found, method, ns, ns
                    )
                }
            },
            TypeError::MethodNotCalled { struct_name, method, .. } => write!(
                f, "{}'{}' is a method of struct '{}', not a field; call it with '.{}()'",
                self.location(), method, display_name(struct_name), method
            ),
            TypeError::NoSuchMethod { struct_name, method, is_field, .. } => {
                if *is_field {
                    write!(
                        f, "{}'{}' is a field of struct '{}', not a method (fields cannot be called)",
                        self.location(), method, display_name(struct_name)
                    )
                } else {
                    write!(
                        f, "{}struct '{}' has no method '{}'",
                        self.location(), display_name(struct_name), method
                    )
                }
            }
            TypeError::NoSuchField { struct_name, field, .. } => write!(
                f, "{}struct '{}' has no field '{}'",
                self.location(), display_name(struct_name), field
            ),
        }
    }
}

impl TypeError {
    /// the same fact in the friendly tone (`#> friendly-terminal`)
    pub fn friendly(&self) -> String {
        let hint = match self {
            TypeError::ShadowedDeclaration { .. } => spg_lexer::HINT_SHADOW,
            TypeError::UnreachableCode { .. } => spg_lexer::HINT_UNREACHABLE,
            _ => spg_lexer::HINT_TYPES,
        };
        spg_lexer::friendly_sentence(&self.to_string(), hint)
    }
}
