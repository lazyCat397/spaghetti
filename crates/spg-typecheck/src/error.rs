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
    UndefinedVariable { name: String, pos: Pos, suggestion: Option<String> },

    /// callee isn't a function (or isn't defined at all)
    UndefinedFunction { name: String, pos: Pos, suggestion: Option<String> },

    /// `int::foo`: the type exists, the property doesn't
    UnknownTypeProperty { ty: String, name: String, pos: Pos },

    /// `const X: i8 = 300;`. the literal does not fit the declared sized type. wrap is not for literals.
    IntOutOfRange { value: i64, ty: Ty, pos: Pos },

    /// a literal above i64::MAX where it is not a u64 (`target` is what it was expected to be, int if none)
    UIntOutOfRange { value: u64, target: Ty, pos: Pos },

    /// `@into.i8(300)` with a literal: the value does not fit. known at compile time. the `w` name wraps it
    IntoOutOfRange { value: i64, ty: Ty, pos: Pos },

    /// `const X: f32 = 1e39;`: the literal is beyond f32's range
    FloatOutOfRange { value: f64, pos: Pos },

    /// `type(x) == "strnig"`: a string compared with `type(...)` must name a type the language has
    UnknownTypeName { name: String, pos: Pos },

    /// `ps.run(...)` / `http.get(...)` in a file with no `attach #ps`.
    NeedsAttach { lib: String, pos: Pos },

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
    /// `som.calls = 5` from outside `namespace som`: its statics are read-only from there
    NamespaceReadOnly { namespace: String, member: String, pos: Pos },
    /// `tool.hidden` from another file, where `hidden` is a member without `public`
    NamespaceMemberPrivate { namespace: String, member: String, pos: Pos },
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
    /// `fn string.len(...)` where `len` is already a method of `string` (built-in or shorthand)
    ValueMethodClash { type_name: String, name: String, pos: Pos },
    /// the same value method declared twice in the program (e.g. by two installed modules)
    ValueMethodDuplicate { type_name: String, name: String, pos: Pos },
    /// `function Cor.Vermelho()` where `Vermelho` is already a variant of enum `Cor`
    AssociatedNameIsVariant { type_name: String, name: String, pos: Pos },

    /// `throw 5`: exceptions carry a string. not a number. not a struct. a string.
    ThrowNotString { found: Ty, pos: Pos },
    /// `catch (e: Point)`: what `catch` gets is the thrown string. Point is not a string.
    CatchNotString { found: Ty, pos: Pos },
    /// `s.nope()` / `xs.nope()` / `m.nope()`: no builtin of `@str`/`@array`/`@map` with that
    /// name takes the value first. nope.
    NoSuchNativeMethod { found: Ty, method: String, pos: Pos },
    /// `"x".tag()` where `tag` is a method of this type declared in another file without `public`
    ValueMethodNotVisible { type_name: String, method: String, pos: Pos },
    /// `f(cor: "azul")` where `f` is a builtin, a lambda or a value: named args need a declared `fn`
    NamedArgNotAllowed { name: String, pos: Pos },
    /// `x.int()` on `x: int or string`: a string member has no implicit conversion to int (it needs parsing)
    UnionConversion { target: String, member: String, var: String, pos: Pos },
    /// `m().int()` on a union value that is not a variable: the conversion needs a name to test
    UnionConversionNeedsName { target: String, pos: Pos },
    /// `desenha(cores: 1)` where `desenha` has no param `cores`
    UnknownArgument { function: String, name: String, pos: Pos },
    /// `desenha(cor: "a", cor: "b")`, or `cor` given both by position and by name
    DuplicateArgument { function: String, name: String, pos: Pos },
    /// a call with named args that leaves out a param without a default value
    MissingArgument { function: String, name: String, pos: Pos },

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
            | TypeError::UnknownTypeProperty { pos, .. }
            | TypeError::IntOutOfRange { pos, .. }
            | TypeError::IntoOutOfRange { pos, .. }
            | TypeError::UIntOutOfRange { pos, .. }
            | TypeError::FloatOutOfRange { pos, .. }
            | TypeError::UnknownTypeName { pos, .. }
            | TypeError::NeedsAttach { pos, .. }
            | TypeError::NotCallable { pos, .. }
            | TypeError::CaptureAssign { pos, .. }
            | TypeError::ArityMismatch { pos, .. }
            | TypeError::ArgTypeMismatch { pos, .. }
            | TypeError::InvalidOperandTypes { pos, .. }
            | TypeError::NotIterable { pos, .. }
            | TypeError::RangeNotInt { pos, .. }
            | TypeError::RangeZeroStep { pos, .. }
            | TypeError::DuplicateDeclaration { pos, .. }
            | TypeError::NamespaceReadOnly { pos, .. }
            | TypeError::NamespaceMemberPrivate { pos, .. }
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
            | TypeError::ValueMethodClash { pos, .. }
            | TypeError::ValueMethodDuplicate { pos, .. }
            | TypeError::MethodNotCalled { pos, .. }
            | TypeError::NoSuchNativeMethod { pos, .. }
            | TypeError::ValueMethodNotVisible { pos, .. }
            | TypeError::NamedArgNotAllowed { pos, .. }
            | TypeError::UnknownArgument { pos, .. }
            | TypeError::UnionConversion { pos, .. }
            | TypeError::UnionConversionNeedsName { pos, .. }
            | TypeError::DuplicateArgument { pos, .. }
            | TypeError::MissingArgument { pos, .. }
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
            TypeError::NeedsAttach { lib, .. } => write!(
                f, "{}'{}' is an installable library, not a variable: it needs `attach #{}` at the top of the file, and is used as {}.member(...)",
                self.location(), lib, lib, lib
            ),
            TypeError::UnknownTypeName { name, .. } => write!(
                f, "{}unknown type name '{}' in a type() test: the names are int, float, bool, string, array, map, fn, task, none, or a struct or enum declared in this file",
                self.location(), name
            ),
            TypeError::NotCallable { .. } => write!(
                f, "{}only functions can be called, and this value is not a function",
                self.location()
            ),
            TypeError::CaptureAssign { name, .. } => write!(
                f, "{}a lambda cannot assign '{}': it is a copy of a variable from the function it was written in",
                self.location(), shown(name)
            ),
            TypeError::FloatOutOfRange { value, .. } => write!(
                f, "{}the value {} does not fit f32 (its largest value is about 3.4e38)",
                self.location(), value
            ),
            TypeError::UIntOutOfRange { value, target, .. } => write!(
                f, "{}the value {} does not fit {}: a number above 9223372036854775807 needs a u64",
                self.location(), value, target
            ),
            TypeError::IntoOutOfRange { value, ty, .. } => write!(
                f, "{}the value {} does not fit {} (range {}): use @into.{}w to wrap it into the range",
                self.location(), value, ty, sized_range(ty), ty
            ),
            TypeError::IntOutOfRange { value, ty, .. } => write!(
                f, "{}the value {} does not fit {} (range {})",
                self.location(), value, ty, sized_range(ty)
            ),
            TypeError::UnknownTypeProperty { ty, name, .. } => write!(
                f, "{}type '{}' has no property '{}'",
                self.location(), ty, name
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
            TypeError::NamespaceMemberPrivate { namespace, member, .. } => write!(
                f, "{}'{}.{}' is private to its file: mark it 'public' inside 'namespace {}' to use it here",
                self.location(), namespace, member, namespace
            ),
            TypeError::NamespaceReadOnly { namespace, member, .. } => {
                let (ns, member) = (spg_parser::ast::display_name(namespace), spg_parser::ast::display_name(member));
                write!(
                    f, "{}namespace '{}' is read-only from outside: '{}.{}' cannot be assigned",
                    self.location(), ns, ns, member
                )
            }
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
            TypeError::ValueMethodDuplicate { type_name, name, .. } => write!(
                f,
                "{}'{}.{}' is declared more than once: a value method has one definition in the program",
                self.location(), type_name, name
            ),
            TypeError::ValueMethodClash { type_name, name, .. } => write!(
                f,
                "{}'{}.{}' clashes with the built-in method '{}' of {}: pick another name",
                self.location(), type_name, name, name, type_name
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
            TypeError::UnionConversion { target, member, var, .. } => write!(
                f, "{}'{}' may be {}, and a {} has no implicit conversion to {}: test its type first and convert that case on its own",
                self.location(), var, member, member, target
            ),
            TypeError::UnionConversionNeedsName { target, .. } => write!(
                f, "{}converting a union value to {} needs it in a variable first: store it in one, then convert it",
                self.location(), target
            ),
            TypeError::NamedArgNotAllowed { name, .. } => write!(
                f, "{}'{}:' names an argument, but only a function or method declared with 'fn' takes named arguments; give this argument by position",
                self.location(), name
            ),
            TypeError::UnknownArgument { function, name, .. } => write!(
                f, "{}'{}' has no parameter named '{}'",
                self.location(), shown(function), name
            ),
            TypeError::DuplicateArgument { function, name, .. } => write!(
                f, "{}'{}' gets argument '{}' more than once in this call",
                self.location(), shown(function), name
            ),
            TypeError::MissingArgument { function, name, .. } => write!(
                f, "{}'{}' needs argument '{}': it has no default value, so this call must give it",
                self.location(), shown(function), name
            ),
            TypeError::ValueMethodNotVisible { type_name, method, .. } => write!(
                f, "{}'{}' is a method of {} declared in another file, and it is not exported: add 'public' in front of its declaration to call it from here",
                self.location(), method, type_name
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
            TypeError::NeedsAttach { .. } => spg_lexer::HINT_ATTACH,
            TypeError::ArityMismatch { .. } => spg_lexer::HINT_ARITY,
            TypeError::UndefinedVariable { .. }
            | TypeError::UndefinedFunction { .. }
            | TypeError::UnknownTypeProperty { .. }
            | TypeError::UndefinedStruct { .. }
            | TypeError::NoSuchField { .. }
            | TypeError::NoSuchMethod { .. }
            | TypeError::NoSuchNativeMethod { .. }
            | TypeError::ValueMethodNotVisible { .. }
            | TypeError::UnknownArgument { .. }
            | TypeError::NamedArgNotAllowed { .. }
            | TypeError::DuplicateArgument { .. }
            | TypeError::MissingArgument { .. }
            | TypeError::NoSuchVariant { .. }
            | TypeError::DuplicateDeclaration { .. }
            | TypeError::AssociatedOnUnknownType { .. }
            | TypeError::UnknownTypeName { .. }
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
            if spg_libs::module(ns).is_some_and(|m| m.kind == spg_libs::Kind::Attach) {
                return rest.to_string();
            }
        }
    }
    base.to_string()
}

/// the range of a sized int type, for the message. the numbers come from `Ty::sized_bounds`.
pub fn sized_range(ty: &crate::typed_ast::Ty) -> String {
    match ty.sized_bounds() {
        Some((lo, hi)) => format!("{}..={}", lo, hi),
        None => String::new(),
    }
}

/// the symbol a binary operator is written with (`Sub` -> `-`), for messages a person reads
fn op_symbol(op: &str) -> &str {
    match op {
        "Add" => "+",
        "Sub" => "-",
        "Mul" => "*",
        "Div" => "/",
        "Mod" => "%",
        "Eq" => "==",
        "NotEq" => "!=",
        "Lt" => "<",
        "Gt" => ">",
        "LtEq" => "<=",
        "GtEq" => ">=",
        "And" => "and",
        "Or" => "or",
        other => other,
    }
}

/// `a int`, `an int`, `an i8`: the article a type name takes
fn with_article(ty: &str) -> String {
    match ty.chars().next() {
        Some('a' | 'e' | 'i' | 'o' | 'u' | 'A' | 'E' | 'I' | 'O' | 'U') => format!("an {}", ty),
        _ => format!("a {}", ty),
    }
}

/// classic edit distance (insert, delete, replace each cost 1)
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        prev = cur;
    }
    prev[b.len()]
}

/// the candidate closest to `name`, when it's close enough to be a plausible typo.
/// the limit scales with the name: short names only accept one edit, long ones up to three.
pub fn closest_name<'a>(name: &str, candidates: impl Iterator<Item = &'a String>) -> Option<String> {
    let limit = (name.chars().count() / 3).clamp(1, 3);
    let mut best: Option<(usize, &String)> = None;
    for c in candidates {
        // skip internal names (mangled '#', '::') and the name itself
        if c == name || c.contains('#') || c.contains("::") {
            continue;
        }
        let d = edit_distance(name, c);
        if d <= limit && best.map_or(true, |(bd, bc)| d < bd || (d == bd && c < bc)) {
            best = Some((d, c));
        }
    }
    best.map(|(_, c)| c.clone())
}

/// a sample value of a type, for the "for example" hints. unknown types get a placeholder.
fn sample_value(ty: &str) -> &'static str {
    match ty {
        "int" => "0",
        "float" => "0.0",
        "string" => "\"text\"",
        "bool" => "true",
        t if t.starts_with("array") => "[]",
        _ => "...",
    }
}

impl TypeError {
    /// stable code for the frame header (`error[E0001]`). unlisted errors share E0000 for now.
    pub fn code(&self) -> &'static str {
        match self {
            TypeError::UndefinedVariable { .. } => "E0001",
            TypeError::UndefinedFunction { .. } => "E0002",
            TypeError::ArityMismatch { .. } => "E0003",
            TypeError::ArgTypeMismatch { .. } => "E0004",
            TypeError::AnnotationMismatch { .. } => "E0005",
            TypeError::ReturnTypeMismatch { .. } => "E0006",
            TypeError::InvalidOperandTypes { .. } => "E0007",
            TypeError::NotIterable { .. } => "E0008",
            TypeError::MissingReturn { .. } => "E0009",
            TypeError::IntOutOfRange { .. } => "E0010",
            _ => self.code_extra(),
        }
    }

    /// the reason behind the error, in one sentence. none when the fact already says it all.
    pub fn why(&self) -> Option<String> {
        let text = match self {
            TypeError::AnnotationMismatch { .. } => "a variable with a declared type only accepts values of that type".to_string(),
            TypeError::ReturnTypeMismatch { .. } => "the return type is part of the function's signature, so every return must match it".to_string(),
            TypeError::MissingReturn { .. } => "a function with a return type must return a value on every path".to_string(),
            TypeError::ArityMismatch { .. } => "the call passes a different number of arguments than the function declares".to_string(),
            TypeError::ArgTypeMismatch { .. } => "each parameter has one type, and there is no implicit conversion to another type".to_string(),
            TypeError::InvalidOperandTypes { op, .. } => format!("the operator '{}' is not defined for these two types", op_symbol(op)),
            TypeError::NotIterable { .. } => "'for ... in' walks over arrays and ranges only".to_string(),
            TypeError::IntOutOfRange { .. } => "the value does not fit the declared sized type".to_string(),
            TypeError::UndefinedVariable { .. } => "a name must be declared (with let, const or as a parameter) before it is used".to_string(),
            TypeError::UndefinedFunction { .. } => "only functions declared in this file, or in an attached library, can be called by name".to_string(),
            _ => return self.why_extra(),
        };
        Some(text)
    }

    /// concrete ways to fix it, built from the names and types in THIS error. empty when there's no better advice than `hint()`.
    pub fn help(&self) -> Vec<String> {
        match self {
            TypeError::AnnotationMismatch { expected, found, .. } => vec![
                format!("make it {}, for example: {}", with_article(&expected.to_string()), sample_value(&expected.to_string())),
                format!("or change the declared type to {}", found),
            ],
            TypeError::ReturnTypeMismatch { expected, found, .. } => vec![
                format!("return {} value, for example: return {}", with_article(&expected.to_string()), sample_value(&expected.to_string())),
                format!("or change the function's return type to {}", found),
            ],
            TypeError::MissingReturn { expected, .. } => vec![
                format!("end the function with a return of type {}, for example: return {}", expected, sample_value(&expected.to_string())),
                "if an if-branch skips the return, give the else branch a return too".to_string(),
            ],
            TypeError::ArityMismatch { function_name, expected, .. } => vec![
                format!("call '{}' with exactly {} argument(s)", shown(function_name), expected),
                "a parameter with a default value may be left out".to_string(),
            ],
            TypeError::ArgTypeMismatch { expected, .. } => {
                let ty = expected.to_string();
                vec![
                    format!("pass {} here, for example: {}", with_article(&ty), sample_value(&ty)),
                    format!("or convert the value first with @into.{}(value)", ty),
                ]
            }
            TypeError::InvalidOperandTypes { op, .. } => {
                let first = match op.as_str() {
                    "Add" => "'+' adds two numbers or joins two strings: make both sides the same kind".to_string(),
                    "Sub" | "Mul" | "Div" | "Mod" => format!("'{}' works with int or float on both sides", op_symbol(op)),
                    _ => format!("make both sides of '{}' the same type", op_symbol(op)),
                };
                vec![first, "or convert one side explicitly with @into.<type>(value)".to_string()]
            }
            TypeError::NotIterable { .. } => vec![
                "use an array, for example: for x in [1, 2, 3]".to_string(),
                "or a range, for example: for i in 0..10".to_string(),
            ],
            TypeError::IntOutOfRange { ty, .. } => vec![
                format!("pick a value in the range of {}: {}", ty, sized_range(ty)),
                "or use a wider type, such as int (64 bits)".to_string(),
            ],
            TypeError::UndefinedVariable { name, suggestion, .. } => {
                let mut out = Vec::new();
                if let Some(s) = suggestion {
                    out.push(format!("did you mean '{}'?", shown(s)));
                }
                out.push(format!("declare it before this line, for example: let {} = ...", shown(name)));
                if suggestion.is_none() {
                    out.push("check the spelling: names are case-sensitive".to_string());
                }
                out
            }
            TypeError::UndefinedFunction { name, suggestion, .. } if !name.contains('.') => {
                let mut out = Vec::new();
                if let Some(s) = suggestion {
                    out.push(format!("did you mean '{}'?", shown(s)));
                }
                out.push(format!("declare it first, for example: fn {}() {{ ... }}", shown(name)));
                if suggestion.is_none() {
                    out.push("check the spelling: names are case-sensitive".to_string());
                }
                out
            }
            _ => match self.help_extra() {
                extra if !extra.is_empty() => extra,
                _ => match self.hint() {
                    "" => vec![],
                    h => vec![h.to_string()],
                },
            },
        }
    }

    /// the bare fact for the frame header, without the `type error at line ...` prefix
    pub fn fact(&self) -> String {
        match self {
            // the base message keeps its optional-type suffix; only the operator name changes
            TypeError::InvalidOperandTypes { op, .. } => self.message().replacen(
                &format!("operator {} not supported", op),
                &format!("operator '{}' not supported", op_symbol(op)),
                1,
            ),
            _ => self.message(),
        }
    }
}


impl TypeError {
    /// codes for the errors the main table in `code` doesn't name
    fn code_extra(&self) -> &'static str {
        match self {
            TypeError::IfExprBranchMismatch { .. } => "E0011",
            TypeError::MatchSubjectType { .. } => "E0012",
            TypeError::MatchCaseType { .. } => "E0013",
            TypeError::MatchNonExhaustive { .. } => "E0014",
            TypeError::MatchDuplicateCase { .. } => "E0015",
            TypeError::UnknownTypeProperty { .. } => "E0016",
            TypeError::UIntOutOfRange { .. } => "E0018",
            TypeError::IntoOutOfRange { .. } => "E0019",
            TypeError::FloatOutOfRange { .. } => "E0020",
            TypeError::UnknownTypeName { .. } => "E0021",
            TypeError::NeedsAttach { .. } => "E0022",
            TypeError::NotCallable { .. } => "E0023",
            TypeError::CaptureAssign { .. } => "E0024",
            TypeError::RangeNotInt { .. } => "E0025",
            TypeError::RangeZeroStep { .. } => "E0026",
            TypeError::DuplicateDeclaration { .. } => "E0027",
            TypeError::NamespaceReadOnly { .. } => "E0028",
            TypeError::NamespaceMemberPrivate { .. } => "E0029",
            TypeError::ShadowedDeclaration { .. } => "E0030",
            TypeError::UnreachableCode { .. } => "E0031",
            TypeError::NonBoolCondition { .. } => "E0032",
            TypeError::AssignToConst { .. } => "E0033",
            TypeError::IncDecTarget { .. } => "E0034",
            TypeError::GuardedIncDecValue { .. } => "E0035",
            TypeError::GuardReentry { .. } => "E0036",
            TypeError::AssignTypeMismatch { .. } => "E0037",
            TypeError::LoopControlOutsideLoop { .. } => "E0038",
            TypeError::CannotInferReturnType { .. } => "E0039",
            TypeError::NotIndexable { .. } => "E0040",
            TypeError::IndexTypeMismatch { .. } => "E0041",
            TypeError::MapKeyType { .. } => "E0042",
            TypeError::MapKeyMismatch { .. } => "E0043",
            TypeError::MapValueMismatch { .. } => "E0044",
            TypeError::ArrayItemMismatch { .. } => "E0045",
            TypeError::ArrayItemNotExpected { .. } => "E0046",
            TypeError::ArrayFillType { .. } => "E0047",
            TypeError::EmptyMapNeedsType { .. } => "E0048",
            TypeError::NoneNeedsType { .. } => "E0049",
            TypeError::CoalesceNeedsOptional { .. } => "E0050",
            TypeError::VoidValue { .. } => "E0051",
            TypeError::IndexAssignTypeMismatch { .. } => "E0052",
            TypeError::UndefinedStruct { .. } => "E0053",
            TypeError::MissingFields { .. } => "E0054",
            TypeError::UnknownField { .. } => "E0055",
            TypeError::StructFieldTypeMismatch { .. } => "E0056",
            TypeError::ReadOnlyField { .. } => "E0057",
            TypeError::NotAStruct { .. } => "E0058",
            TypeError::NoSuchVariant { .. } => "E0059",
            TypeError::EnumAsValue { .. } => "E0060",
            TypeError::NoSuchField { .. } => "E0061",
            TypeError::NoSuchMethod { .. } => "E0062",
            TypeError::NoMethodsOnType { .. } => "E0063",
            TypeError::AssociatedOnUnknownType { .. } => "E0064",
            TypeError::AssociatedCalledOnValue { .. } => "E0065",
            TypeError::NoAssociatedFunction { .. } => "E0066",
            TypeError::ValueMethodClash { .. } => "E0067",
            TypeError::ValueMethodDuplicate { .. } => "E0068",
            TypeError::AssociatedNameIsVariant { .. } => "E0069",
            TypeError::ThrowNotString { .. } => "E0070",
            TypeError::CatchNotString { .. } => "E0071",
            TypeError::NoSuchNativeMethod { .. } => "E0072",
            TypeError::ValueMethodNotVisible { .. } => "E0073",
            TypeError::NamedArgNotAllowed { .. } => "E0074",
            TypeError::UnionConversion { .. } => "E0075",
            TypeError::UnionConversionNeedsName { .. } => "E0076",
            TypeError::UnknownArgument { .. } => "E0077",
            TypeError::DuplicateArgument { .. } => "E0078",
            TypeError::MissingArgument { .. } => "E0079",
            TypeError::MethodNotCalled { .. } => "E0080",
            _ => "E0000",
        }
    }

    /// the reason for the errors the main table in `why` doesn't cover
    fn why_extra(&self) -> Option<String> {
        let text = match self {
            TypeError::IfExprBranchMismatch { .. } => "both branches of an if that gives a value must give the same type".to_string(),
            TypeError::MatchSubjectType { .. } => "'when' matches on enums, unions and simple values, and this type is none of them".to_string(),
            TypeError::MatchCaseType { .. } => "every case must have the type of the value being matched".to_string(),
            TypeError::MatchNonExhaustive { .. } => "a 'when' over an enum has to say what happens for every variant".to_string(),
            TypeError::MatchDuplicateCase { .. } => "a case can be written only once in a 'when'".to_string(),
            TypeError::UnknownTypeProperty { .. } => "the type has no property with that name".to_string(),
            TypeError::UIntOutOfRange { .. } => "the value does not fit the unsigned type it is used as".to_string(),
            TypeError::IntoOutOfRange { .. } => "the conversion would change the value: it does not fit the target type".to_string(),
            TypeError::FloatOutOfRange { .. } => "the number is too large for a float".to_string(),
            TypeError::UnknownTypeName { .. } => "a type must be a built-in type or a struct or enum declared in the program".to_string(),
            TypeError::NeedsAttach { .. } => "a library's functions are only visible in files that attach it".to_string(),
            TypeError::NotCallable { .. } => "only functions can be called with parentheses".to_string(),
            TypeError::CaptureAssign { .. } => "a function can read a variable from outside it, but can't change it".to_string(),
            TypeError::RangeNotInt { .. } => "a range walks over whole numbers, so both of its bounds must be int".to_string(),
            TypeError::RangeZeroStep { .. } => "a step of zero would never move".to_string(),
            TypeError::DuplicateDeclaration { .. } => "a name can be declared only once in the same scope".to_string(),
            TypeError::NamespaceReadOnly { .. } => "the members of a library can't be changed".to_string(),
            TypeError::NamespaceMemberPrivate { .. } => "this member is not exported by its library".to_string(),
            TypeError::ShadowedDeclaration { .. } => "strict mode forbids a name that hides one from an outer scope".to_string(),
            TypeError::UnreachableCode { .. } => "this code can never run: the block already left before reaching it".to_string(),
            TypeError::NonBoolCondition { .. } => "a condition must be a bool: true or false".to_string(),
            TypeError::AssignToConst { .. } => "a const never changes after it is declared".to_string(),
            TypeError::IncDecTarget { .. } => "++ and -- only work on a variable or a field that holds an int".to_string(),
            TypeError::GuardedIncDecValue { .. } => "a guarded struct's fields can only change inside its own methods".to_string(),
            TypeError::GuardReentry { .. } => "a guard can't be entered again while it is still running".to_string(),
            TypeError::AssignTypeMismatch { .. } => "a variable keeps the type it was declared with".to_string(),
            TypeError::LoopControlOutsideLoop { keyword, .. } => format!("'{}' only means something inside a loop", keyword),
            TypeError::CannotInferReturnType { .. } => "the returns of this function give different types, and no return type is declared".to_string(),
            TypeError::NotIndexable { .. } => "only arrays and maps can be indexed with []".to_string(),
            TypeError::IndexTypeMismatch { .. } => "an array is indexed by int, and a map by its key type".to_string(),
            TypeError::MapKeyType { .. } => "map keys must be int, string, bool or an enum".to_string(),
            TypeError::MapKeyMismatch { .. } => "all keys of one map have the same type".to_string(),
            TypeError::MapValueMismatch { .. } => "all values of one map have the same type".to_string(),
            TypeError::ArrayItemMismatch { .. } => "all items of an array have the same type".to_string(),
            TypeError::ArrayItemNotExpected { .. } => "this array takes items of one type, and the item here is another".to_string(),
            TypeError::ArrayFillType { .. } => "the type of an array comes from its items, and this value can't give one".to_string(),
            TypeError::EmptyMapNeedsType { .. } => "an empty map doesn't say what its keys and values are".to_string(),
            TypeError::NoneNeedsType { .. } => "none doesn't say what type it stands for".to_string(),
            TypeError::CoalesceNeedsOptional { .. } => "'??' only replaces a missing value (none)".to_string(),
            TypeError::VoidValue { .. } => "this expression gives no value (void)".to_string(),
            TypeError::IndexAssignTypeMismatch { .. } => "each item of an array keeps the array's type".to_string(),
            TypeError::UndefinedStruct { .. } => "a struct has to be declared before it is used".to_string(),
            TypeError::MissingFields { .. } => "a struct value has to set every field it declares".to_string(),
            TypeError::UnknownField { .. } => "the struct has no field with that name".to_string(),
            TypeError::StructFieldTypeMismatch { .. } => "each field keeps the type it was declared with".to_string(),
            TypeError::ReadOnlyField { .. } => "this field can't be changed from outside the struct".to_string(),
            TypeError::NotAStruct { .. } => "only structs have fields".to_string(),
            TypeError::NoSuchVariant { .. } => "the enum has no variant with that name".to_string(),
            TypeError::EnumAsValue { .. } => "an enum's name is a type, not a value".to_string(),
            TypeError::NoSuchField { .. } => "the struct has no field with that name".to_string(),
            TypeError::NoSuchMethod { is_field: true, method, .. } => format!("'{}' is a field, not a method", method),
            TypeError::NoSuchMethod { .. } => "the struct has no method with that name".to_string(),
            TypeError::NoMethodsOnType { found, .. } => format!("{} has no methods", found),
            TypeError::AssociatedOnUnknownType { .. } => "only a declared type can have functions attached to it".to_string(),
            TypeError::AssociatedCalledOnValue { .. } => "functions attached to a type are called on the type, not on a value".to_string(),
            TypeError::NoAssociatedFunction { .. } => "the type has no function with that name".to_string(),
            TypeError::ValueMethodClash { .. } => "two definitions would share one name".to_string(),
            TypeError::ValueMethodDuplicate { .. } => "a value method has one definition in the program".to_string(),
            TypeError::AssociatedNameIsVariant { .. } => "this name is already a variant of the enum".to_string(),
            TypeError::ThrowNotString { .. } => "the error of a throw is a string".to_string(),
            TypeError::CatchNotString { .. } => "a catch receives the thrown message, which is a string".to_string(),
            TypeError::NoSuchNativeMethod { found, method, .. } => format!("{} has no method '{}'", found, method),
            TypeError::ValueMethodNotVisible { .. } => "this method is only visible in the file that declares it".to_string(),
            TypeError::NamedArgNotAllowed { .. } => "this function does not name its parameters".to_string(),
            TypeError::UnionConversion { target, member, var, .. } => {
                format!("'{}' may hold a {}, which can't become a {}", var, member, target)
            }
            TypeError::UnionConversionNeedsName { .. } => "a union can only be converted through a variable name".to_string(),
            TypeError::UnknownArgument { function, name, .. } => format!("'{}' has no parameter named '{}'", function, name),
            TypeError::DuplicateArgument { .. } => "an argument can be given only once".to_string(),
            TypeError::MissingArgument { function, name, .. } => format!("'{}' needs the argument '{}'", function, name),
            TypeError::MethodNotCalled { method, .. } => format!("'{}' is a method, so it has to be called with ()", method),
            _ => return None,
        };
        Some(text)
    }

    /// concrete ways to fix the errors the main table in `help` doesn't cover
    fn help_extra(&self) -> Vec<String> {
        match self {
            TypeError::IfExprBranchMismatch { then_ty, else_ty, .. } => vec![
                format!("make the else branch give {}", with_article(&then_ty.to_string())),
                format!("or make the then branch give {}", with_article(&else_ty.to_string())),
            ],
            TypeError::MatchSubjectType { found, .. } => vec![format!("match on an enum or a union value, not {}", with_article(&found.to_string()))],
            TypeError::MatchCaseType { expected, .. } => {
                let ty = expected.to_string();
                vec![format!("write a case of type {}, for example: {}", ty, sample_value(&ty))]
            }
            TypeError::MatchNonExhaustive { missing, .. } => vec![
                format!("add the missing case(s): {}", missing.join(", ")),
                "or add an else branch that catches the rest".to_string(),
            ],
            TypeError::MatchDuplicateCase { value, .. } => vec![format!("remove the second '{}' case", value)],
            TypeError::UnknownTypeProperty { name, .. } => vec![format!("check the spelling of '{}'", name)],
            TypeError::UIntOutOfRange { target, .. } => vec![
                format!("pick a value in the range of {}: {}", target, sized_range(target)),
                "or use a wider unsigned type, such as u64".to_string(),
            ],
            TypeError::IntoOutOfRange { ty, .. } => vec![
                format!("pick a value in the range of {}: {}", ty, sized_range(ty)),
                "or convert to a wider type, such as int (64 bits)".to_string(),
            ],
            TypeError::FloatOutOfRange { .. } => vec!["use a smaller number, or a f32 for less range".to_string()],
            TypeError::UnknownTypeName { name, .. } => vec![
                format!("check the spelling: '{}' is not a type", display_name(name)),
                format!("or declare it first: struct {} {{ ... }}", display_name(name)),
            ],
            TypeError::NeedsAttach { lib, .. } => vec![format!("add 'attach #{}' at the top of the file", lib)],
            TypeError::NotCallable { .. } => vec!["check the name: it holds a value, not a function".to_string()],
            TypeError::CaptureAssign { name, .. } => vec![format!("copy '{}' into a local variable, and change the copy", name)],
            TypeError::RangeNotInt { which, .. } => vec![format!("make the {} bound an int, for example: 0..10", which)],
            TypeError::RangeZeroStep { .. } => vec!["use a step other than 0, for example: 1".to_string()],
            TypeError::DuplicateDeclaration { name, .. } => vec![
                format!("rename one of the two '{}', or remove the second", name),
                format!("or assign to the existing one: {} = ...", name),
            ],
            TypeError::NamespaceReadOnly { namespace, member, .. } => {
                vec![format!("copy it into a variable: let value = {}.{}(...)", namespace, member)]
            }
            TypeError::NamespaceMemberPrivate { namespace, .. } => {
                vec![format!("use one of the public members of {}", namespace)]
            }
            TypeError::ShadowedDeclaration { name, .. } => vec![
                format!("rename '{}' to a new name here", name),
                "or remove '#> strict' from the file".to_string(),
            ],
            TypeError::UnreachableCode { keyword, .. } => vec![format!("remove it, or move it before the {}", keyword)],
            TypeError::NonBoolCondition { found, .. } => vec![format!("compare the value, for example: x != 0 (it is {})", found)],
            TypeError::AssignToConst { name, .. } => vec![format!("declare '{}' with let if it has to change", name)],
            TypeError::IncDecTarget { .. } => vec!["use x += 1 for a value that is not a plain variable".to_string()],
            TypeError::GuardedIncDecValue { struct_name, .. } => {
                vec![format!("call a method of '{}' that makes the change", struct_name)]
            }
            TypeError::GuardReentry { .. } => vec!["move the inner access out of the guard, or compute it first".to_string()],
            TypeError::AssignTypeMismatch { name, expected, .. } => {
                let ty = expected.to_string();
                vec![
                    format!("give '{}' {} value, for example: {}", name, with_article(&ty), sample_value(&ty)),
                    "or declare a new variable with let".to_string(),
                ]
            }
            TypeError::LoopControlOutsideLoop { .. } => vec!["move it inside a for or while loop".to_string()],
            TypeError::CannotInferReturnType { function_name, .. } => {
                vec![format!("declare the return type: fn {}() -> type {{ ... }}", display_name(function_name))]
            }
            TypeError::NotIndexable { found, .. } => vec![format!("index an array or a map, not {}", with_article(&found.to_string()))],
            TypeError::IndexTypeMismatch { .. } => vec!["use an int index, for example: xs[0]".to_string()],
            TypeError::MapKeyType { .. } => vec!["use an int or a string key".to_string()],
            TypeError::MapKeyMismatch { expected, .. } => {
                let ty = expected.to_string();
                vec![format!("use {} key, for example: {}", with_article(&ty), sample_value(&ty))]
            }
            TypeError::MapValueMismatch { expected, .. } => {
                let ty = expected.to_string();
                vec![format!("use {} value, for example: {}", with_article(&ty), sample_value(&ty))]
            }
            TypeError::ArrayItemMismatch { expected, .. } | TypeError::ArrayItemNotExpected { expected, .. } => {
                let ty = expected.to_string();
                vec![format!("use {} item, for example: {}", with_article(&ty), sample_value(&ty))]
            }
            TypeError::ArrayFillType { .. } => vec!["declare the array's type: let xs: array<int> = ...".to_string()],
            TypeError::EmptyMapNeedsType { .. } => vec!["declare the map's type: let m: map<string, int> = ...".to_string()],
            TypeError::NoneNeedsType { .. } => vec!["declare the type: let x: int? = none".to_string()],
            TypeError::CoalesceNeedsOptional { .. } => vec![
                "remove the '??': this value is never none".to_string(),
                "or make the left side optional, such as int?".to_string(),
            ],
            TypeError::VoidValue { .. } => vec!["use a function that returns a value, or call this one on its own line".to_string()],
            TypeError::IndexAssignTypeMismatch { expected, .. } => {
                let ty = expected.to_string();
                vec![format!("store {} value, for example: {}", with_article(&ty), sample_value(&ty))]
            }
            TypeError::UndefinedStruct { name, .. } => vec![
                format!("check the spelling: '{}' is not a struct", display_name(name)),
                format!("or declare it: struct {} {{ ... }}", display_name(name)),
            ],
            TypeError::MissingFields { missing, .. } => vec![format!("set the missing field(s): {}", missing.join(", "))],
            TypeError::UnknownField { field, .. } => vec![format!("check the spelling of '{}'", field)],
            TypeError::StructFieldTypeMismatch { expected, .. } => {
                let ty = expected.to_string();
                vec![format!("give it {} value, for example: {}", with_article(&ty), sample_value(&ty))]
            }
            TypeError::ReadOnlyField { struct_name, field, .. } => {
                vec![format!("change '{}' through a method of '{}'", field, struct_name)]
            }
            TypeError::NotAStruct { found, .. } => vec![format!("check the value: it is {}, not a struct", found)],
            TypeError::NoSuchVariant { variants, .. } => vec![format!("use one of its variants: {}", variants.join(", "))],
            TypeError::EnumAsValue { name, .. } => vec![format!("use one of its variants, for example: {}.<Variant>", display_name(name))],
            TypeError::NoSuchField { field, .. } => vec![format!("check the spelling of '{}'", field)],
            TypeError::NoSuchMethod { method, is_field: true, .. } => vec![format!("drop the parentheses: use '{}' without ()", method)],
            TypeError::NoSuchMethod { .. } => vec!["check the spelling of the method".to_string()],
            TypeError::NoMethodsOnType { found, method, .. } => {
                // a value type (`i8`, `f32`...) gets its methods from `type::methods.add`, not from a struct
                let name = found.to_string();
                if spg_parser::ast::is_value_type_name(&name) {
                    vec![format!("declare it first: {}::methods.add(fn {}() -> {} {{ ... }})", name, method, name)]
                } else {
                    vec!["call it on a struct value".to_string()]
                }
            }
            TypeError::AssociatedOnUnknownType { type_name, .. } => vec![format!("declare the type first: struct {} {{ ... }}", display_name(type_name))],
            TypeError::AssociatedCalledOnValue { type_name, name, .. } => {
                vec![format!("call it on the type: {}.{}(...)", display_name(type_name), display_name(name))]
            }
            TypeError::NoAssociatedFunction { .. } => vec!["check the spelling, and declare the function inside the type's block".to_string()],
            TypeError::ValueMethodClash { name, .. } => vec![format!("rename one of the two '{}'", display_name(name))],
            TypeError::ValueMethodDuplicate { name, .. } => vec![format!("keep one definition of '{}' and remove the other", display_name(name))],
            TypeError::AssociatedNameIsVariant { .. } => vec!["rename the function".to_string()],
            TypeError::ThrowNotString { .. } => vec!["throw a string, for example: throw \"message\"".to_string()],
            TypeError::CatchNotString { .. } => vec!["catch the message into a string variable".to_string()],
            TypeError::NoSuchNativeMethod { .. } => vec!["check the spelling of the method".to_string()],
            TypeError::ValueMethodNotVisible { method, .. } => vec![format!("call '{}' from the file that declares it", method)],
            TypeError::NamedArgNotAllowed { .. } => vec!["pass the argument by position, without the 'name:'".to_string()],
            TypeError::UnionConversion { var, .. } => vec![format!("check which member '{}' holds first, with when", var)],
            TypeError::UnionConversionNeedsName { .. } => vec!["store the value in a variable first, then convert it".to_string()],
            TypeError::UnknownArgument { .. } => vec!["check the spelling of the argument name".to_string()],
            TypeError::DuplicateArgument { .. } => vec!["remove one of the two".to_string()],
            TypeError::MissingArgument { function, name, .. } => vec![format!("pass it: {}({}: ...)", display_name(function), name)],
            TypeError::MethodNotCalled { method, .. } => vec![format!("add the parentheses: ...{}()", method)],
            _ => vec![],
        }
    }
}
