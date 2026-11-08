//! Fatal type errors. the checker collects ALL of them in one pass
//! instead of stopping at the first one. matches the language's rigid,
//! no-silent-fallback philosophy, but still dumps every mistake at once
//! instead of a fix-rerun-fix loop. you're welcome, user.

use spg_parser::ast::{display_name, Pos};
use crate::typed_ast::Ty;
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum TypeError {
    /// `let x: int = "text"`. the annotation and the value disagree. classic.
    AnnotationMismatch { expected: Ty, found: Ty, pos: Pos },

    /// function says `-> int` but a `return` hands back something else. liar.
    ReturnTypeMismatch { expected: Ty, found: Ty, pos: Pos },

    /// a path through a non-void function has no `return`. the function just gives up.
    MissingReturn { function_name: String, expected: Ty, pos: Pos },

    /// the two branches of an if-EXPRESSION disagree. pick a type, any type.
    IfExprBranchMismatch { then_ty: Ty, else_ty: Ty, pos: Pos },

    /// `when (x)` where `x` isn't an int, a string or an enum. not matchable, sorry.
    MatchSubjectType { found: Ty, pos: Pos },
    /// a case value whose type doesn't match the matched value's
    MatchCaseType { expected: Ty, found: Ty, pos: Pos },
    /// an enum match that skips variants and has no `else`. we noticed.
    MatchNonExhaustive { missing: Vec<String>, pos: Pos },
    MatchDuplicateCase { value: String, pos: Pos },

    /// identifier used but never declared anywhere you can see. ghost variable.
    UndefinedVariable { name: String, pos: Pos },

    /// callee isn't a function (or isn't defined at all)
    UndefinedFunction { name: String, pos: Pos },

    /// an old builtin name (`sqrt`, `toUpper`, `clock`, ...) from before
    /// namespaces. `replacement` is the new name. we moved stuff around.
    RemovedBuiltin { name: String, replacement: String, pos: Pos },

    /// `math.sqrt(...)`, `list.push(...)`, `transform.toInt(...)`, `time.run()`.
    /// the namespace spelling from before the `@` days. nostalgia not included.
    /// `replacement` is the current name, when there is one.
    OldNamespace { name: String, replacement: Option<String>, pos: Pos },

    /// `ps.run(...)` / `http.get(...)` in a file with no `install #ps`.
    NeedsInstall { lib: String, pos: Pos },

    /// a call whose callee isn't a plain function name (`p.x()`, `f()()`,
    /// `(a + b)()`). only named functions and builtins can be called. don't get fancy.
    NotCallable { pos: Pos },

    /// a lambda assigns a variable of the function it lives in. captured values are copies, so this does nothing useful. watch out.
    CaptureAssign { name: String, pos: Pos },

    /// wrong number of args. count your fingers.
    ArityMismatch { function_name: String, expected: usize, found: usize, pos: Pos },

    /// arg type doesn't match the parameter. read the signature, man.
    ArgTypeMismatch { function_name: String, param_index: usize, expected: Ty, found: Ty, pos: Pos },

    /// operator used on types it doesn't support (e.g. "a" - 1). strings don't subtract.
    InvalidOperandTypes { op: String, left: Ty, right: Ty, pos: Pos },

    /// `for x in y` where `y` isn't an array. nothing to loop over.
    NotIterable { found: Ty, pos: Pos },

    /// `for i in a..b step s` where `a`, `b` or `s` isn't an int
    RangeNotInt { which: &'static str, found: Ty, pos: Pos },

    /// `for i in a..b step 0`: a literal zero step never finishes. infinite loop, caught.
    RangeZeroStep { pos: Pos },

    /// re-declaring a name already taken in the same scope. pick a new name.
    DuplicateDeclaration { name: String, pos: Pos },
    /// `#> strict`: a name hiding another one from an outer scope. shadowing, but grumpy.
    ShadowedDeclaration { name: String, pos: Pos },
    /// `#> strict`: code after `return` / `break` / `continue` / `throw`. dead code. bye.
    UnreachableCode { keyword: &'static str, pos: Pos },

    /// an `if`/`while` condition isn't Bool. no truthy nonsense here.
    NonBoolCondition { found: Ty, pos: Pos },

    /// `x = value` where `x` was declared `const`. read the keyword, friend.
    AssignToConst { name: String, pos: Pos },

    /// `++`/`--` on something that is not a variable, an array element or a struct field
    IncDecTarget { pos: Pos },

    /// `obj.field++` used as a value on a struct with `guard` blocks: a write inside an
    /// expression cannot be wrapped in the guard check. as a statement it can.
    GuardedIncDecValue { struct_name: String, pos: Pos },

    /// a guard of `struct_name` can reach a write to `struct_name`. `path` lists the
    /// steps from the guard to that write, for the message only.
    GuardReentry { struct_name: String, path: Vec<String>, pos: Pos },

    /// `x = value` where the value's type disagrees with `x`'s type
    AssignTypeMismatch { name: String, expected: Ty, found: Ty, pos: Pos },

    /// `break`/`continue` outside any loop. nothing to break out of.
    LoopControlOutsideLoop { keyword: String, pos: Pos },

    /// no trailing `-> Type` given, and the function's `return`
    /// statements disagree on type, so nothing could be inferred.
    /// unlike ReturnTypeMismatch (which compares to a type the
    /// user DID declare), this one fires when there was nothing to compare
    /// against in the first place. the function is undecided. so are we.
    CannotInferReturnType { function_name: String, found_types: Vec<Ty>, pos: Pos },

    /// `expr[index]` or `expr[index] = ...` where `expr` isn't an array
    NotIndexable { found: Ty, pos: Pos },

    /// an array index that isn't `int` (e.g. `arr["x"]`). no string indexes, sorry.
    IndexTypeMismatch { found: Ty, pos: Pos },

    /// a map key type that can't be hashed. only int, string, bool and enums.
    MapKeyType { found: Ty, pos: Pos },

    /// `m[k]` / `@map.has(m, k)` / a literal entry with a key of the wrong type
    MapKeyMismatch { expected: Ty, found: Ty, pos: Pos },

    /// a value of the wrong type for the map (`m[k] = v`, literal entry, `@map.get` fallback)
    MapValueMismatch { expected: Ty, found: Ty, pos: Pos },

    /// `[1, 2.5]`: every item must match the first item's type. no mixing.
    ArrayItemMismatch { expected: Ty, found: Ty, pos: Pos },
    /// an item that does not fit the array type the context expects (`let a: array<int> = ["x"]`)
    ArrayItemNotExpected { expected: Ty, found: Ty, pos: Pos },

    /// `@array.fill(n, x)` with an `x` that isn't int, float, bool or string
    ArrayFillType { found: Ty, pos: Pos },

    /// `let m = [:]` with nothing saying what the map holds. the map is a mystery.
    EmptyMapNeedsType { pos: Pos },

    /// `let x = none`: nothing says which optional type this is. none of your business, apparently. it is.
    NoneNeedsType { pos: Pos },

    /// `a ?? b` where `a` isn't an optional. nothing to default.
    CoalesceNeedsOptional { found: Ty, pos: Pos },

    /// `let x = <expr>` / `const` where the value is void (e.g. an `if`
    /// whose branches only call print). nothing to store. shoutout to print.
    VoidValue { pos: Pos },

    /// `arr[i] = value` where `value`'s type disagrees with the array's
    /// element type. same story, different array.
    IndexAssignTypeMismatch { expected: Ty, found: Ty, pos: Pos },

    /// `Name { .. }` where `Name` was never declared with `struct`. made-up structs, no.
    UndefinedStruct { name: String, pos: Pos },

    /// a struct literal missing some of its fields. fill in the blanks.
    MissingFields { struct_name: String, missing: Vec<String>, pos: Pos },

    /// a struct literal names a field the struct doesn't have. invented fields, no.
    UnknownField { struct_name: String, field: String, pos: Pos },

    /// a struct literal field's value doesn't match that field's
    /// declared type. same song.
    StructFieldTypeMismatch { struct_name: String, field: String, expected: Ty, found: Ty, pos: Pos },

    /// `obj.field = value` on a field a native object only lets the program read (`win.width`)
    ReadOnlyField { struct_name: String, field: String, pos: Pos },

    /// `expr.field` where `expr` isn't a struct at all. fields need structs.
    NotAStruct { found: Ty, pos: Pos },

    /// `Color.Purple` where enum `Color` has no such variant. purple isn't a color here.
    NoSuchVariant { enum_name: String, variant: String, variants: Vec<String>, pos: Pos },

    /// the bare enum name used as a value (`let x = Color`). the enum isn't a value, silly.
    EnumAsValue { name: String, pos: Pos },

    /// `expr.field` where `expr` IS a struct but doesn't have `field`
    NoSuchField { struct_name: String, field: String, pos: Pos },

    /// `p.f()` where struct `P` has no method `f`. `is_field` is true when
    /// `f` IS a field of `P` (so the hint can say fields aren't callable)
    NoSuchMethod { struct_name: String, method: String, is_field: bool, pos: Pos },

    /// `x.f()` where `x` isn't a struct at all (an int, a string, an array...)
    NoMethodsOnType { found: Ty, method: String, pos: Pos },

    /// `function Foo.bar()` where `Foo` is not a struct of this program
    AssociatedOnUnknownType { type_name: String, name: String, pos: Pos },
    /// `p.bar()` where `bar` is an associated function: it is called as `Type.bar()`
    AssociatedCalledOnValue { type_name: String, name: String, pos: Pos },
    /// `Type.bar()` where `Type` is a struct with no associated function `bar`
    NoAssociatedFunction { type_name: String, name: String, pos: Pos },
    /// `fn string.name(...)` whose first parameter is not a `string`: the value it is called on must come first
    ValueMethodNeedsReceiver { type_name: String, name: String, pos: Pos },
    /// `fn string.len(...)` where `len` is already a method of `string` (built-in or shorthand)
    ValueMethodClash { type_name: String, name: String, pos: Pos },
    /// `function Cor.Vermelho()` where `Vermelho` is already a variant of enum `Cor`
    AssociatedNameIsVariant { type_name: String, name: String, pos: Pos },

    /// `throw 5`: exceptions carry a string. not a number. not a struct. a string.
    ThrowNotString { found: Ty, pos: Pos },
    /// `catch (e: Point)`: what `catch` gets is the thrown string. Point is not a string.
    CatchNotString { found: Ty, pos: Pos },
    /// `s.nope()` / `xs.nope()` / `m.nope()`: no builtin of `@str`/`@array`/`@map` with that
    /// name takes the value first. nope.
    NoSuchNativeMethod { found: Ty, method: String, pos: Pos },

    /// `p.f` (no call) where `f` is a METHOD, not a field. add the parentheses.
    MethodNotCalled { struct_name: String, method: String, pos: Pos },
}

impl TypeError {
    /// the `Pos` carried by whichever variant this is. one place that
    /// knows every variant has exactly one `pos` field, so `Display`
    /// below doesn't need its own copy of this same 25-arm match just
    /// to find the file/line/col to print. yes, 25 arms. it hurts.
    pub fn pos(&self) -> &Pos {
        match self {
            TypeError::AnnotationMismatch { pos, .. }
            | TypeError::ReturnTypeMismatch { pos, .. }
            | TypeError::MissingReturn { pos, .. }
            | TypeError::IfExprBranchMismatch { pos, .. }
            | TypeError::MatchSubjectType { pos, .. }
            | TypeError::MatchCaseType { pos, .. }
            | TypeError::MatchNonExhaustive { pos, .. }
            | TypeError::MatchDuplicateCase { pos, .. }
            | TypeError::UndefinedVariable { pos, .. }
            | TypeError::UndefinedFunction { pos, .. }
            | TypeError::RemovedBuiltin { pos, .. }
            | TypeError::OldNamespace { pos, .. }
            | TypeError::NeedsInstall { pos, .. }
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
            | TypeError::IncDecTarget { pos }
            | TypeError::GuardedIncDecValue { pos, .. }
            | TypeError::GuardReentry { pos, .. }
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
            | TypeError::ArrayItemNotExpected { pos, .. }
            | TypeError::IndexAssignTypeMismatch { pos, .. }
            | TypeError::UndefinedStruct { pos, .. }
            | TypeError::MissingFields { pos, .. }
            | TypeError::UnknownField { pos, .. }
            | TypeError::StructFieldTypeMismatch { pos, .. }
            | TypeError::ReadOnlyField { pos, .. }
            | TypeError::NotAStruct { pos, .. }
            | TypeError::NoSuchVariant { pos, .. }
            | TypeError::EnumAsValue { pos, .. }
            | TypeError::NoSuchField { pos, .. }
            | TypeError::NoSuchMethod { pos, .. }
            | TypeError::NoMethodsOnType { pos, .. }
            | TypeError::AssociatedOnUnknownType { pos, .. }
            | TypeError::AssociatedCalledOnValue { pos, .. }
            | TypeError::NoAssociatedFunction { pos, .. }
            | TypeError::AssociatedNameIsVariant { pos, .. }
            | TypeError::ValueMethodNeedsReceiver { pos, .. }
            | TypeError::ValueMethodClash { pos, .. }
            | TypeError::MethodNotCalled { pos, .. }
            | TypeError::NoSuchNativeMethod { pos, .. }
            | TypeError::ThrowNotString { pos, .. }
            | TypeError::CatchNotString { pos, .. } => pos,
        }
    }

    /// the full `"type error ...: "` prefix every message below starts
    /// with. spg-modules stamps a `Pos.file` on every statement of every
    /// program it resolves (single-file ones too), so in
    /// practice `file` is always `Some` here, so every error names its
    /// file, not just its line/column. the `None` arm only exists for
    /// `Pos` values built by something other than spg-modules
    /// (a hand-built `Program` in a test, say). it falls back to the
    /// old line/column-only wording instead of printing a placeholder. good.
    /// the message alone, without the `type error in <file> at line X,
    /// column Y: ` prefix. for consumers (the language server) that carry
    /// the position separately. the LSP has its own opinions.
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
        // an optional where a plain value was needed. say how to get out of it.
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
                self.location(), shown(function_name), expected
            ),
            TypeError::MatchSubjectType { found, .. } => write!(
                f, "{}'when' can only match an int, a string or an enum, but this value is {}",
                self.location(), found
            ),
            TypeError::MatchCaseType { expected, found, .. } => write!(
                f, "{}this case is {}, but the matched value is {}",
                self.location(), found, expected
            ),
            TypeError::MatchDuplicateCase { value, .. } => write!(
                f,
                "'when' already has a case for {}; the first one is the only one that can run, remove this one",
                value
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
                self.location(), shown(name)
            ),
            TypeError::NeedsInstall { lib, .. } => write!(
                f, "{}'{}' is an installable library, not a variable: it needs `install #{}` at the top of the file, and is used as {}.member(...)",
                self.location(), lib, lib, lib
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
                self.location(), shown(name)
            ),
            TypeError::UndefinedFunction { name, .. } => match name.split_once('.') {
                // `@math.foo`: the namespace exists, the member doesn't. sad.
                Some((ns, member)) => write!(
                    f, "{}namespace '{}' has no member '{}'",
                    self.location(), ns, member
                ),
                None => write!(
                    f, "{}undefined function '{}'",
                    self.location(), shown(name)
                ),
            },
            TypeError::RemovedBuiltin { name, replacement, .. } => write!(
                f, "{}'{}' is no longer a global function; use '{}'",
                self.location(), shown(name), replacement
            ),
            TypeError::ArityMismatch { function_name, expected, found, .. } => write!(
                f, "{}'{}' expects {} argument(s), got {}",
                self.location(), shown(function_name), expected, found
            ),
            TypeError::ArgTypeMismatch { function_name, param_index, expected, found, .. } => write!(
                f, "{}'{}' argument #{} expects {}, got {}",
                self.location(), shown(function_name), param_index + 1, expected, found
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
                self.location(), shown(name)
            ),
            TypeError::ShadowedDeclaration { name, .. } => write!(
                f, "{}'{}' hides a name from an outer scope (not allowed with #> strict)",
                self.location(), shown(name)
            ),
            TypeError::UnreachableCode { keyword, .. } => write!(
                f, "{}code after '{}' can never run (not allowed with #> strict)",
                self.location(), keyword
            ),
            TypeError::NonBoolCondition { found, .. } => write!(
                f, "{}condition must be bool, found {}",
                self.location(), found
            ),
            TypeError::GuardReentry { struct_name, path, .. } => write!(
                f, "{}a guard of '{}' can write to '{}' again: {}. a guard cannot change its own struct",
                self.location(), struct_name, struct_name, path.join(" -> ")
            ),
            TypeError::GuardedIncDecValue { struct_name, .. } => write!(
                f, "{}'++' and '--' on a field of struct '{}' (it has guards) must be a statement, like `obj.field++;`",
                self.location(), struct_name
            ),
            TypeError::IncDecTarget { .. } => write!(
                f, "{}'++' and '--' work on a variable, an array element or a struct field (like `i++` or `a[i]++`)",
                self.location()
            ),
            TypeError::AssignToConst { name, .. } => write!(
                f, "{}cannot assign to '{}' — it's declared as const",
                self.location(), shown(name)
            ),
            TypeError::AssignTypeMismatch { name, expected, found, .. } => write!(
                f, "{}cannot assign {} to '{}' — it's declared as {}",
                self.location(), found, shown(name), expected
            ),
            TypeError::LoopControlOutsideLoop { keyword, .. } => write!(
                f, "{}'{}' used outside of a loop",
                self.location(), keyword
            ),
            TypeError::CannotInferReturnType { function_name, found_types, .. } => write!(
                f, "{}cannot infer return type of '{}' — its return statements disagree ({}); add an explicit '-> Type'",
                self.location(), shown(function_name),
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
            TypeError::ArrayItemNotExpected { expected, found, .. } => write!(
                f, "{}this array's items are {}, found {}",
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
                self.location(), shown(name)
            ),
            TypeError::MissingFields { struct_name, missing, .. } => write!(
                f, "{}struct literal for '{}' is missing field(s): {}",
                self.location(), shown(struct_name), missing.join(", ")
            ),
            TypeError::UnknownField { struct_name, field, .. } => write!(
                f, "{}struct '{}' has no field '{}'",
                self.location(), shown(struct_name), field
            ),
            TypeError::ReadOnlyField { struct_name, field, .. } => write!(
                f, "{}cannot assign to '{}.{}' — it is read-only, the program can only read it",
                self.location(), struct_name, field
            ),
            TypeError::StructFieldTypeMismatch { struct_name, field, expected, found, .. } => write!(
                f, "{}field '{}' of struct '{}' expects {}, found {}",
                self.location(), field, shown(struct_name), expected, found
            ),
            TypeError::NoSuchVariant { enum_name, variant, variants, .. } => write!(
                f, "{}enum '{}' has no variant '{}' (variants: {})",
                self.location(), shown(enum_name), variant, variants.join(", ")
            ),
            TypeError::EnumAsValue { name, .. } => write!(
                f, "{}'{}' is an enum, not a value; pick a variant ('{}.<Variant>') or loop over it ('for x in {}')",
                self.location(), shown(name), shown(name), shown(name)
            ),
            TypeError::NotAStruct { found, .. } => write!(
                f, "{}type {} has no fields (not a struct)",
                self.location(), found
            ),
            TypeError::AssociatedOnUnknownType { type_name, name, .. } => write!(
                f,
                "associated function '{}' needs a struct named '{}' declared in this program",
                name, type_name
            ),
            TypeError::AssociatedCalledOnValue { type_name, name, .. } => write!(
                f,
                "'{}' is an associated function of '{}': call it as {}.{}(), not on a value",
                name, type_name, type_name, name
            ),
            TypeError::ValueMethodNeedsReceiver { type_name, name, .. } => write!(
                f,
                "method '{}.{}' needs a first parameter of type {}: the value it is called on",
                type_name, name, type_name
            ),
            TypeError::ValueMethodClash { type_name, name, .. } => write!(
                f,
                "'{}.{}' clashes with the built-in method '{}' of {}: pick another name",
                type_name, name, name, type_name
            ),
            TypeError::AssociatedNameIsVariant { type_name, name, .. } => write!(
                f,
                "associated function '{}.{}' clashes with the variant '{}' of enum '{}': pick another name",
                type_name, name, name, type_name
            ),
            TypeError::NoAssociatedFunction { type_name, name, .. } => {
                write!(f, "'{}' has no associated function '{}'", type_name, name)
            }
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
                self.location(), method, shown(struct_name), method
            ),
            TypeError::NoSuchMethod { struct_name, method, is_field, .. } => {
                if *is_field {
                    write!(
                        f, "{}'{}' is a field of struct '{}', not a method (fields cannot be called)",
                        self.location(), method, shown(struct_name)
                    )
                } else {
                    write!(
                        f, "{}struct '{}' has no method '{}'",
                        self.location(), shown(struct_name), method
                    )
                }
            }
            TypeError::NoSuchField { struct_name, field, .. } => write!(
                f, "{}struct '{}' has no field '{}'",
                self.location(), shown(struct_name), field
            ),
        }
    }
}

impl TypeError {
    /// the same fact, friendly tone (`#> friendly-terminal`). the nice version.
    pub fn friendly(&self) -> String {
        spg_lexer::friendly_sentence(&self.to_string(), self.hint())
    }

    /// the friendly line for the CLI list: where it is, what's wrong, what to do.
    /// `in file.spg, line 2, column 1: declared type is int but value is string. ...`
    pub fn friendly_at(&self) -> String {
        let pos = self.pos();
        let place = match &pos.file {
            Some(file) => format!("in {}, line {}, column {}", file, pos.line, pos.col),
            None => format!("at line {}, column {}", pos.line, pos.col),
        };
        let fact = self.message();
        let fact = fact.trim_end_matches('.');
        format!("{}: {}. {}", place, fact, self.hint())
    }

    /// the advice that fits this kind of error. a wrong hint is worse than none
    fn hint(&self) -> &'static str {
        match self {
            TypeError::ShadowedDeclaration { .. } => spg_lexer::HINT_SHADOW,
            TypeError::UnreachableCode { .. } => spg_lexer::HINT_UNREACHABLE,
            TypeError::AssignToConst { .. } => spg_lexer::HINT_CONST,
            TypeError::NeedsInstall { .. } => spg_lexer::HINT_INSTALL,
            TypeError::ArityMismatch { .. } => spg_lexer::HINT_ARITY,
            TypeError::UndefinedVariable { .. }
            | TypeError::UndefinedFunction { .. }
            | TypeError::UndefinedStruct { .. }
            | TypeError::RemovedBuiltin { .. }
            | TypeError::OldNamespace { .. }
            | TypeError::NoSuchField { .. }
            | TypeError::NoSuchMethod { .. }
            | TypeError::NoSuchNativeMethod { .. }
            | TypeError::NoSuchVariant { .. }
            | TypeError::DuplicateDeclaration { .. }
            | TypeError::AssociatedOnUnknownType { .. }
            | TypeError::NoAssociatedFunction { .. } => spg_lexer::HINT_NAMES,
            _ => spg_lexer::HINT_TYPES,
        }
    }
}

/// how a function name reads in a message: no mangling, and `regex.matches` for an installed
/// library (only the core ones are written with `@`, so `@regex.matches` would be wrong)
fn shown(name: &str) -> String {
    let base = display_name(name);
    if let Some(rest) = base.strip_prefix('@') {
        if let Some((ns, _)) = rest.split_once('.') {
            if spg_libs::module(ns).is_some_and(|m| m.kind == spg_libs::Kind::Install) {
                return rest.to_string();
            }
        }
    }
    base.to_string()
}
