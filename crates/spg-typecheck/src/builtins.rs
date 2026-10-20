//! builtin function signatures for the typechecker. the menu.
//!
//! builtins get checked against this table, not `Checker::functions`,
//! so their params can use `Ty::Any`/`Ty::AnyArray` without messing up
//! `FunctionSig`, which mirrors the user's always-explicit `Param` syntax.
//!
//! a name can have several signatures (overloads). e.g. `@into.string`
//! takes int, float or bool, and array builtins take any element
//! type via `Ty::AnyArray`. call-site checking tries each signature in
//! order and takes the first exact arity+type match. it's a fixed,
//! finite table (users can't extend it), so a linear scan is fine.
//!
//! every builtin also carries a `BuiltinId`, the number the compiler and VM
//! use to find it in bytecode. the single source of truth,
//! so adding a builtin here is
//! the one place you write a signature by hand. pain, but contained.

use crate::typed_ast::Ty;

/// identifies a builtin in bytecode. `spg-compiler` emits this
/// inside `OpCode::CallBuiltin`, and `spg-vm` dispatches on it directly,
/// so the numbering only has to hold inside one compiled program
/// (no bytecode gets saved between builds, so nobody cares. yet.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum BuiltinId {
    Print,

    // ---- tasks. `t.wait()` only, the checker builds it (the result type depends on T) ----
    TaskWait,
    /// `@task.all(tasks)`: waits for every task, gives back their results. the checker types it
    TaskAll,

    // ---- @json (text in, text out, pain) ----

    // ---- strings (utf-8 says hi) ----
    Len,
    ToUpper,
    ToLower,
    Trim,
    Split,
    Substring,
    CharAt,
    StrContains,
    StartsWith,
    EndsWith,
    Replace,
    StrIndexOf,
    Repeat,
    IntToString,
    FloatToString,
    BoolToString,
    /// `@into.string(enumValue)`: chosen by the checker (no table entry, it's special)
    EnumToString,
    /// `@into.string(x)` for any other type: the same text `print` shows. consistent.
    AnyToString,

    // ---- arrays (the big family) ----
    ArrayLenBuiltin,
    ArrayPush,
    ArrayPop,
    ArrayContains,
    ArrayIndexOf,
    ArraySlice,
    ArrayReverse,
    ArraySort,
    ArrayJoin,
    /// `@array.range(a, b)` / `@array.range(a, b, step)`: ints from `a` up to
    /// (not including) `b`, as an array. end is exclusive, we've said it.
    ArrayRange2,
    ArrayRange3,
    /// INTERNAL (no name in the table): the compiler emits this for a
    /// `for .. in a..b step s` whose step isn't a literal. returns the step
    /// unchanged, or throws the zero-step runtime error. your step, your problem.
    RangeStepCheck,

    // ---- @file (the disk, again) ----

    // ---- @sys (the OS, again) ----
    SysArgs,
    SysExit,
    SysEnv,
    SysPlatform,
    SysCwd,
    SysClear,

    /// a function of a native library (`ps`, `http`, ...): the global index in the registry of
    /// `spg-libs`. its signature and its code live in that library's own file.
    Lib(u16),
    SysReadLine,
    SysReadAll,

    // ---- more @str (still going) ----
    PadStart,
    PadStartFill,
    PadEnd,
    PadEndFill,
    StrLastIndexOf,
    TrimStart,
    TrimEnd,

    // ---- more @array (send help) ----
    ArrayInsert,
    ArrayRemove,
    ArrayClear,
    ArrayFill,
    ArrayConcat,
    ArraySumInt,
    ArraySumFloat,

    // ---- more @math (the calculator is on fire) ----
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Atan2,
    Exp,
    Log,
    Log2,
    Log10,
    ToRadians,
    ToDegrees,
    SignInt,
    SignFloat,
    ClampInt,
    ClampFloat,
    TauDecimals,
    TauDefault,

    // ---- maps (@map, the hash situation) ----
    MapHas,
    MapGet,
    MapRemove,
    MapLen,
    MapKeys,
    MapValues,
    MapKeysOf,

    // ---- math (floats, trust no one) ----
    Abs,
    AbsFloat,
    Floor,
    Ceil,
    Round,
    Sqrt,
    PowFloat,
    MinInt,
    MaxInt,
    MinFloat,
    MaxFloat,
    Random,
    /// `@math.pi(n)` / `@math.e(n)`: the constant rounded to `n` decimal places
    PiDecimals,
    EDecimals,
    /// `@math.pi()` / `@math.e()`: the constant at the default precision (2 decimals). pi is 3.14 for us now. shrug.
    PiDefault,
    EDefault,

    // ---- type conversion (@into, the great liar) ----
    IntToFloat,
    FloatToInt,
    ParseInt,
    ParseFloat,

    // ---- I/O (print and friends) ----
    ReadLine,
    ReadLinePrompt,
    PrintError,

    // ---- time ----
}

pub struct BuiltinSig {
    pub id: BuiltinId,
    pub params: Vec<Ty>,
    pub return_type: Ty,
}

/// every overload for a given builtin name, in the order call-site
/// checking should try them. first match wins.
impl BuiltinId {
    /// the `@array.*` builtins that take an element (or two arrays) whose type
    /// must match the array's own. the checker validates them itself
    /// (`check_array_call`), because the table only knows `Any` / `AnyArray`.
    pub fn is_array_checked(self) -> bool {
        matches!(
            self,
            BuiltinId::ArrayPush
                | BuiltinId::ArrayContains
                | BuiltinId::ArrayIndexOf
                | BuiltinId::ArrayInsert
                | BuiltinId::ArrayFill
                | BuiltinId::ArrayConcat
        )
    }

    /// the `@map.*` builtins. their key/value types come from the map argument,
    /// so the checker types them itself (`check_map_call`), not from the table
    pub fn is_map_builtin(self) -> bool {
        matches!(
            self,
            BuiltinId::MapHas
                | BuiltinId::MapGet
                | BuiltinId::MapRemove
                | BuiltinId::MapLen
                | BuiltinId::MapKeys
                | BuiltinId::MapValues
                | BuiltinId::MapKeysOf
        )
    }
}

/// `"(string, map<string, string>, int) -> HttpResponse"` -> its parameter types and return type.
/// the types a native library can name: int, float, bool, string, void, array<T>, map<K, V>,
/// or the name of one of its structs.
pub fn parse_sig(sig: &str) -> (Vec<Ty>, Ty) {
    let (params, ret) = sig.split_once("->").expect("a library signature has a `->`");
    let params = params.trim().trim_start_matches('(').trim_end_matches(')');
    let params = if params.trim().is_empty() { Vec::new() } else { split_top_level(params).into_iter().map(parse_ty).collect() };
    (params, parse_ty(ret))
}

fn parse_ty(text: &str) -> Ty {
    let t = text.trim();
    match t {
        "int" => Ty::Int,
        "float" => Ty::Float,
        "bool" => Ty::Bool,
        "string" => Ty::String,
        "void" => Ty::Void,
        "any" => Ty::Any,
        _ => {
            if let Some(inner) = t.strip_prefix("array<").and_then(|r| r.strip_suffix('>')) {
                Ty::Array(Box::new(parse_ty(inner)))
            } else if let Some(inner) = t.strip_prefix("map<").and_then(|r| r.strip_suffix('>')) {
                let parts = split_top_level(inner);
                assert_eq!(parts.len(), 2, "map<K, V> in a library signature");
                Ty::Map(Box::new(parse_ty(parts[0])), Box::new(parse_ty(parts[1])))
            } else {
                Ty::Named(t.to_string())
            }
        }
    }
}

/// splits at the commas that are not inside `<...>`
fn split_top_level(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut depth, mut start) = (0usize, 0usize);
    for (i, c) in text.char_indices() {
        match c {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(&text[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&text[start..]);
    parts
}

pub fn builtin_signatures() -> std::collections::HashMap<String, Vec<BuiltinSig>> {
    let mut m: std::collections::HashMap<String, Vec<BuiltinSig>> = std::collections::HashMap::new();

    let mut add = |name: &str, sig: BuiltinSig| {
        m.entry(name.to_string()).or_default().push(sig);
    };

    // ---- I/O (print and friends) ----
    add("print", BuiltinSig { id: BuiltinId::Print, params: vec![Ty::Any], return_type: Ty::Void });
    add(
        "printError",
        BuiltinSig { id: BuiltinId::PrintError, params: vec![Ty::Any], return_type: Ty::Void },
    );
    add("readLine", BuiltinSig { id: BuiltinId::ReadLine, params: vec![], return_type: Ty::String });
    add("readLine", BuiltinSig { id: BuiltinId::ReadLinePrompt, params: vec![Ty::String], return_type: Ty::String });

    // ---- strings (utf-8 says hi) ----
    add("@str.len", BuiltinSig { id: BuiltinId::Len, params: vec![Ty::String], return_type: Ty::Int });
    add(
        "@str.toUpper",
        BuiltinSig { id: BuiltinId::ToUpper, params: vec![Ty::String], return_type: Ty::String },
    );
    add(
        "@str.toLower",
        BuiltinSig { id: BuiltinId::ToLower, params: vec![Ty::String], return_type: Ty::String },
    );
    add("@str.trim", BuiltinSig { id: BuiltinId::Trim, params: vec![Ty::String], return_type: Ty::String });
    add(
        "@str.split",
        BuiltinSig {
            id: BuiltinId::Split,
            params: vec![Ty::String, Ty::String],
            return_type: Ty::Array(Box::new(Ty::String)),
        },
    );
    add(
        "@str.substring",
        BuiltinSig {
            id: BuiltinId::Substring,
            params: vec![Ty::String, Ty::Int, Ty::Int],
            return_type: Ty::String,
        },
    );
    add(
        "@str.charAt",
        BuiltinSig { id: BuiltinId::CharAt, params: vec![Ty::String, Ty::Int], return_type: Ty::String },
    );
    add(
        "@str.contains",
        BuiltinSig {
            id: BuiltinId::StrContains,
            params: vec![Ty::String, Ty::String],
            return_type: Ty::Bool,
        },
    );
    // `contains` overloaded for array<T> too (element type checked as Any
    // at this layer. the VM compares values structurally at runtime)
    add(
        "@array.contains",
        BuiltinSig { id: BuiltinId::ArrayContains, params: vec![Ty::AnyArray, Ty::Any], return_type: Ty::Bool },
    );
    add(
        "@str.startsWith",
        BuiltinSig {
            id: BuiltinId::StartsWith,
            params: vec![Ty::String, Ty::String],
            return_type: Ty::Bool,
        },
    );
    add(
        "@str.endsWith",
        BuiltinSig { id: BuiltinId::EndsWith, params: vec![Ty::String, Ty::String], return_type: Ty::Bool },
    );
    add(
        "@str.replace",
        BuiltinSig {
            id: BuiltinId::Replace,
            params: vec![Ty::String, Ty::String, Ty::String],
            return_type: Ty::String,
        },
    );
    add(
        "@str.indexOf",
        BuiltinSig {
            id: BuiltinId::StrIndexOf,
            params: vec![Ty::String, Ty::String],
            return_type: Ty::Int,
        },
    );
    add(
        "@array.indexOf",
        BuiltinSig { id: BuiltinId::ArrayIndexOf, params: vec![Ty::AnyArray, Ty::Any], return_type: Ty::Int },
    );
    add(
        "@str.repeat",
        BuiltinSig { id: BuiltinId::Repeat, params: vec![Ty::String, Ty::Int], return_type: Ty::String },
    );
    // `@into.string` takes int, float or bool. that's the whole list.
    add(
        "@into.string",
        BuiltinSig { id: BuiltinId::IntToString, params: vec![Ty::Int], return_type: Ty::String },
    );
    add(
        "@into.string",
        BuiltinSig { id: BuiltinId::FloatToString, params: vec![Ty::Float], return_type: Ty::String },
    );
    add(
        "@into.string",
        BuiltinSig { id: BuiltinId::BoolToString, params: vec![Ty::Bool], return_type: Ty::String },
    );

    // ---- arrays (the big family) ----
    // `len` overloaded for array<T>, next to the string version above
    add("@array.len", BuiltinSig { id: BuiltinId::ArrayLenBuiltin, params: vec![Ty::AnyArray], return_type: Ty::Int });
    add(
        "@array.push",
        BuiltinSig { id: BuiltinId::ArrayPush, params: vec![Ty::AnyArray, Ty::Any], return_type: Ty::Void },
    );
    add("@array.pop", BuiltinSig { id: BuiltinId::ArrayPop, params: vec![Ty::AnyArray], return_type: Ty::Any });
    add(
        "@array.range",
        BuiltinSig {
            id: BuiltinId::ArrayRange2,
            params: vec![Ty::Int, Ty::Int],
            return_type: Ty::Array(Box::new(Ty::Int)),
        },
    );
    add(
        "@array.range",
        BuiltinSig {
            id: BuiltinId::ArrayRange3,
            params: vec![Ty::Int, Ty::Int, Ty::Int],
            return_type: Ty::Array(Box::new(Ty::Int)),
        },
    );
    add(
        "@array.slice",
        BuiltinSig {
            id: BuiltinId::ArraySlice,
            params: vec![Ty::AnyArray, Ty::Int, Ty::Int],
            return_type: Ty::AnyArray,
        },
    );
    add(
        "@array.reverse",
        BuiltinSig { id: BuiltinId::ArrayReverse, params: vec![Ty::AnyArray], return_type: Ty::AnyArray },
    );
    add("@array.sort", BuiltinSig { id: BuiltinId::ArraySort, params: vec![Ty::AnyArray], return_type: Ty::AnyArray });
    add(
        "@array.join",
        BuiltinSig {
            id: BuiltinId::ArrayJoin,
            params: vec![Ty::Array(Box::new(Ty::String)), Ty::String],
            return_type: Ty::String,
        },
    );

    // ---- @file: the disk. every I/O failure is a catchable runtime error. the disk is not nice. ----
    let s = Ty::String;
    let strs = || Ty::Array(Box::new(Ty::String));

    // ---- @json: text <-> values. reads work on the text by path, writes take any value ----
    // @file lives in spg-libs (src/file.rs): its signatures come from the registry below

    // ---- @sys: the process the program lives in ----
    add("@sys.args", BuiltinSig { id: BuiltinId::SysArgs, params: vec![], return_type: strs() });
    add("@sys.exit", BuiltinSig { id: BuiltinId::SysExit, params: vec![Ty::Int], return_type: Ty::Void });
    add("@sys.env", BuiltinSig { id: BuiltinId::SysEnv, params: vec![s.clone(), s.clone()], return_type: s.clone() });
    add("@sys.platform", BuiltinSig { id: BuiltinId::SysPlatform, params: vec![], return_type: s.clone() });
    add("@sys.cwd", BuiltinSig { id: BuiltinId::SysCwd, params: vec![], return_type: s.clone() });
    add("@sys.clear", BuiltinSig { id: BuiltinId::SysClear, params: vec![], return_type: Ty::Void });

    // ---- native libraries (ps, http, ...): the registry in spg-libs gives name, signature and docs ----
    for (index, module, f) in spg_libs::all_functions() {
        let (params, return_type) = parse_sig(f.sig);
        add(&format!("@{}.{}", module.name, f.name), BuiltinSig { id: BuiltinId::Lib(index), params, return_type });
    }
    add("@sys.readLine", BuiltinSig { id: BuiltinId::SysReadLine, params: vec![], return_type: s.clone() });
    add("@sys.readAll", BuiltinSig { id: BuiltinId::SysReadAll, params: vec![], return_type: s.clone() });

    // ---- more @str (still going) ----
    // padStart/padEnd(s, width [, fill]): grow `s` to `width` chars with `fill` (default " "),
    // repeated and cut as needed. a shorter width or empty fill leaves `s` alone.
    add("@str.padStart", BuiltinSig { id: BuiltinId::PadStart, params: vec![Ty::String, Ty::Int], return_type: Ty::String });
    add(
        "@str.padStart",
        BuiltinSig { id: BuiltinId::PadStartFill, params: vec![Ty::String, Ty::Int, Ty::String], return_type: Ty::String },
    );
    add("@str.padEnd", BuiltinSig { id: BuiltinId::PadEnd, params: vec![Ty::String, Ty::Int], return_type: Ty::String });
    add(
        "@str.padEnd",
        BuiltinSig { id: BuiltinId::PadEndFill, params: vec![Ty::String, Ty::Int, Ty::String], return_type: Ty::String },
    );
    // index (in chars) of the LAST occurrence, or -1. last, not first. pay attention.
    add(
        "@str.lastIndexOf",
        BuiltinSig { id: BuiltinId::StrLastIndexOf, params: vec![Ty::String, Ty::String], return_type: Ty::Int },
    );
    add("@str.trimStart", BuiltinSig { id: BuiltinId::TrimStart, params: vec![Ty::String], return_type: Ty::String });
    add("@str.trimEnd", BuiltinSig { id: BuiltinId::TrimEnd, params: vec![Ty::String], return_type: Ty::String });

    add("@array.insert", BuiltinSig { id: BuiltinId::ArrayInsert, params: vec![Ty::AnyArray, Ty::Int, Ty::Any], return_type: Ty::Void });
    add("@array.remove", BuiltinSig { id: BuiltinId::ArrayRemove, params: vec![Ty::AnyArray, Ty::Int], return_type: Ty::Any });
    add("@array.clear", BuiltinSig { id: BuiltinId::ArrayClear, params: vec![Ty::AnyArray], return_type: Ty::Void });
    // placeholders: `check_array_call` types these two (fill -> array<T> of the value, concat -> the arrays' type)
    add("@array.fill", BuiltinSig { id: BuiltinId::ArrayFill, params: vec![Ty::Int, Ty::Any], return_type: Ty::AnyArray });
    add("@array.concat", BuiltinSig { id: BuiltinId::ArrayConcat, params: vec![Ty::AnyArray, Ty::AnyArray], return_type: Ty::AnyArray });
    add(
        "@array.sum",
        BuiltinSig { id: BuiltinId::ArraySumInt, params: vec![Ty::Array(Box::new(Ty::Int))], return_type: Ty::Int },
    );
    add(
        "@array.sum",
        BuiltinSig { id: BuiltinId::ArraySumFloat, params: vec![Ty::Array(Box::new(Ty::Float))], return_type: Ty::Float },
    );

    // ---- maps ----
    // the parameter/return types below are placeholders (`Any`/`AnyMap`): the
    // the checker validates the key and fallback against the map's own types
    add("@map.has", BuiltinSig { id: BuiltinId::MapHas, params: vec![Ty::AnyMap, Ty::Any], return_type: Ty::Bool });
    add(
        "@map.get",
        BuiltinSig { id: BuiltinId::MapGet, params: vec![Ty::AnyMap, Ty::Any, Ty::Any], return_type: Ty::Any },
    );
    add("@map.remove", BuiltinSig { id: BuiltinId::MapRemove, params: vec![Ty::AnyMap, Ty::Any], return_type: Ty::Bool });
    add("@map.len", BuiltinSig { id: BuiltinId::MapLen, params: vec![Ty::AnyMap], return_type: Ty::Int });
    add("@map.keys", BuiltinSig { id: BuiltinId::MapKeys, params: vec![Ty::AnyMap], return_type: Ty::AnyArray });
    add("@map.values", BuiltinSig { id: BuiltinId::MapValues, params: vec![Ty::AnyMap], return_type: Ty::AnyArray });
    add("@map.keysOf", BuiltinSig { id: BuiltinId::MapKeysOf, params: vec![Ty::AnyMap, Ty::Any], return_type: Ty::AnyArray });

    // ---- math (floats, trust no one) ----
    add("@math.abs", BuiltinSig { id: BuiltinId::Abs, params: vec![Ty::Int], return_type: Ty::Int });
    add("@math.abs", BuiltinSig { id: BuiltinId::AbsFloat, params: vec![Ty::Float], return_type: Ty::Float });
    add("@math.floor", BuiltinSig { id: BuiltinId::Floor, params: vec![Ty::Float], return_type: Ty::Int });
    add("@math.ceil", BuiltinSig { id: BuiltinId::Ceil, params: vec![Ty::Float], return_type: Ty::Int });
    add("@math.round", BuiltinSig { id: BuiltinId::Round, params: vec![Ty::Float], return_type: Ty::Int });
    add("@math.sqrt", BuiltinSig { id: BuiltinId::Sqrt, params: vec![Ty::Float], return_type: Ty::Float });
    add(
        "@math.pow",
        BuiltinSig { id: BuiltinId::PowFloat, params: vec![Ty::Float, Ty::Float], return_type: Ty::Float },
    );
    add("@math.min", BuiltinSig { id: BuiltinId::MinInt, params: vec![Ty::Int, Ty::Int], return_type: Ty::Int });
    add("@math.max", BuiltinSig { id: BuiltinId::MaxInt, params: vec![Ty::Int, Ty::Int], return_type: Ty::Int });
    add(
        "@math.min",
        BuiltinSig { id: BuiltinId::MinFloat, params: vec![Ty::Float, Ty::Float], return_type: Ty::Float },
    );
    add(
        "@math.max",
        BuiltinSig { id: BuiltinId::MaxFloat, params: vec![Ty::Float, Ty::Float], return_type: Ty::Float },
    );
    add("@math.random", BuiltinSig { id: BuiltinId::Random, params: vec![], return_type: Ty::Float });
    // constants: `pi(n)` / `e(n)` give n decimal places (rounded). without an
    // argument, 2 decimals (3.14 / 2.72). the `(int)`
    // form is registered first, so a wrong arg reports against it. order matters.
    add("@math.pi", BuiltinSig { id: BuiltinId::PiDecimals, params: vec![Ty::Int], return_type: Ty::Float });
    add("@math.pi", BuiltinSig { id: BuiltinId::PiDefault, params: vec![], return_type: Ty::Float });
    add("@math.e", BuiltinSig { id: BuiltinId::EDecimals, params: vec![Ty::Int], return_type: Ty::Float });
    add("@math.e", BuiltinSig { id: BuiltinId::EDefault, params: vec![], return_type: Ty::Float });

    // trig (radians), exponentials and logs: float -> float. the
    // domain errors follow IEEE (`log(0.0)` is -inf, `asin(2.0)` is NaN). same as sqrt. math being math.
    for (name, id) in [
        ("@math.sin", BuiltinId::Sin),
        ("@math.cos", BuiltinId::Cos),
        ("@math.tan", BuiltinId::Tan),
        ("@math.asin", BuiltinId::Asin),
        ("@math.acos", BuiltinId::Acos),
        ("@math.atan", BuiltinId::Atan),
        ("@math.exp", BuiltinId::Exp),
        ("@math.log", BuiltinId::Log),
        ("@math.log2", BuiltinId::Log2),
        ("@math.log10", BuiltinId::Log10),
        ("@math.toRadians", BuiltinId::ToRadians),
        ("@math.toDegrees", BuiltinId::ToDegrees),
    ] {
        add(name, BuiltinSig { id, params: vec![Ty::Float], return_type: Ty::Float });
    }
    add(
        "@math.atan2",
        BuiltinSig { id: BuiltinId::Atan2, params: vec![Ty::Float, Ty::Float], return_type: Ty::Float },
    );
    // -1, 0 or 1 (a NaN gives 0. sure, why not.)
    add("@math.sign", BuiltinSig { id: BuiltinId::SignInt, params: vec![Ty::Int], return_type: Ty::Int });
    add("@math.sign", BuiltinSig { id: BuiltinId::SignFloat, params: vec![Ty::Float], return_type: Ty::Int });
    // clamp(x, low, high): x limited to [low, high]. low > high is a runtime error
    add(
        "@math.clamp",
        BuiltinSig { id: BuiltinId::ClampInt, params: vec![Ty::Int, Ty::Int, Ty::Int], return_type: Ty::Int },
    );
    add(
        "@math.clamp",
        BuiltinSig { id: BuiltinId::ClampFloat, params: vec![Ty::Float, Ty::Float, Ty::Float], return_type: Ty::Float },
    );
    add("@math.tau", BuiltinSig { id: BuiltinId::TauDecimals, params: vec![Ty::Int], return_type: Ty::Float });
    add("@math.tau", BuiltinSig { id: BuiltinId::TauDefault, params: vec![], return_type: Ty::Float });

    // ---- @into: the three conversions (the target type is the member name) ----
    // `@into.int`: float → int truncates toward zero. string → int parses
    // (and blows up at runtime with InvalidNumberFormat if it's garbage)
    add("@into.int", BuiltinSig { id: BuiltinId::FloatToInt, params: vec![Ty::Float], return_type: Ty::Int });
    add("@into.int", BuiltinSig { id: BuiltinId::ParseInt, params: vec![Ty::String], return_type: Ty::Int });
    add("@into.float", BuiltinSig { id: BuiltinId::IntToFloat, params: vec![Ty::Int], return_type: Ty::Float });
    add("@into.float", BuiltinSig { id: BuiltinId::ParseFloat, params: vec![Ty::String], return_type: Ty::Float });

    // ---- time ----

    // ---- tasks ----
    add("@task.all", BuiltinSig { id: BuiltinId::TaskAll, params: vec![Ty::AnyArray], return_type: Ty::AnyArray });

    // ---- the one global shortcut: `len` (@str.len / @array.len). the only cheat code. ----
    add("len", BuiltinSig { id: BuiltinId::Len, params: vec![Ty::String], return_type: Ty::Int });
    add("len", BuiltinSig { id: BuiltinId::ArrayLenBuiltin, params: vec![Ty::AnyArray], return_type: Ty::Int });

    m
}

/// old flat builtin name → its `@namespace` replacement, so a program written
/// before namespaces gets "use '@math.sqrt'" instead of a bare "undefined
/// function". `len` is absent on purpose. it stays global. the rebel.
pub fn migrated_builtin(old: &str) -> Option<&'static str> {
    Some(match old {
        // strings
        "toUpper" => "@str.toUpper",
        "toLower" => "@str.toLower",
        "trim" => "@str.trim",
        "split" => "@str.split",
        "substring" => "@str.substring",
        "charAt" => "@str.charAt",
        "startsWith" => "@str.startsWith",
        "endsWith" => "@str.endsWith",
        "replace" => "@str.replace",
        "repeat" => "@str.repeat",
        // shared between string and array: depends on the argument
        "contains" => "@str.contains / @array.contains",
        "indexOf" => "@str.indexOf / @array.indexOf",
        // arrays
        "push" => "@array.push",
        "pop" => "@array.pop",
        "slice" => "@array.slice",
        "reverse" => "@array.reverse",
        "sort" => "@array.sort",
        "join" => "@array.join",
        // math
        "abs" => "@math.abs",
        "floor" => "@math.floor",
        "ceil" => "@math.ceil",
        "round" => "@math.round",
        "sqrt" => "@math.sqrt",
        "pow" => "@math.pow",
        "min" => "@math.min",
        "max" => "@math.max",
        "random" => "@math.random",
        // conversions
        "toString" => "@into.string",
        "intToFloat" => "@into.float",
        "floatToInt" => "@into.int",
        "parseInt" => "@into.int",
        "parseFloat" => "@into.float",
        // time
        "clock" => "@time.unix() (wall clock, ms) or @time.elapsed() (readable time since the program started)",
        _ => return None,
    })
}

/// the previous namespace spelling (`math.sqrt`, `string.len`, `list.push`,
/// `transform.toInt`, `time.run`, ...) → the current `@` name. only returns a
/// name that exists in the table, so the hint is always right.
pub fn renamed_namespace_call(old_ns: &str, member: &str) -> Option<String> {
    let new = match (old_ns, member) {
        ("transform", "toString") => "@into.string".to_string(),
        ("transform", "toFloat") | ("transform", "parseFloat") => "@into.float".to_string(),
        ("transform", "toInt") | ("transform", "parseInt") => "@into.int".to_string(),
        ("time", "run") => "@time.elapsed".to_string(),
        ("string", m) => format!("@str.{}", m),
        ("list", m) => format!("@array.{}", m),
        ("math", m) => format!("@math.{}", m),
        ("time", m) => format!("@time.{}", m),
        _ => return None,
    };
    builtin_signatures().contains_key(&new).then_some(new)
}

/// the namespaces whose functions can be called as methods: `x.fn(...)` is
/// `@ns.fn(x, ...)` whenever the first param of `@ns.fn` takes `x`
pub const METHOD_NAMESPACES: [&str; 5] = ["str", "array", "map", "math", "into"];

/// builtins that never act as methods, even with an int/float first param:
/// constants, a random source, and the two builders whose first
/// arg is a count or a bound, not a receiver. not everything is a method.
pub const NOT_METHOD_BUILTINS: [&str; 6] = [
    "@math.pi",
    "@math.e",
    "@math.tau",
    "@math.random",
    "@array.range",
    "@array.fill",
];

/// the namespace (from `METHOD_NAMESPACES`) whose function `method` takes a value
/// of type `ty` as its first arg: `x.abs()` for an int is `@math.abs(x)`.
/// `None` when no such function exists. a namespace listed earlier wins, and
/// the table has no name shared by two namespaces for the same receiver type. no dupes. please.
pub fn method_namespace(ty: &Ty, method: &str) -> Option<&'static str> {
    let table = builtin_signatures();
    METHOD_NAMESPACES.iter().copied().find(|ns| {
        let name = format!("@{}.{}", ns, method);
        !NOT_METHOD_BUILTINS.contains(&name.as_str())
            && table.get(&name).map_or(false, |overloads| {
                overloads
                    .iter()
                    .any(|sig| sig.params.first().map_or(false, |p| crate::checker::ty_matches(p, ty)))
            })
    })
}

/// every method a value of type `ty` has, sorted by name, as
/// `(name, params after the receiver, return type)`. each name is taken
/// from the first overload that takes `ty` first, so `x.min(y)` on an int
/// shows the int overload. the LSP uses this to show number methods. helpful.
pub fn methods_of(ty: &Ty) -> Vec<(String, Vec<Ty>, Ty)> {
    let mut out: Vec<(String, Vec<Ty>, Ty)> = Vec::new();
    for (name, overloads) in builtin_signatures() {
        let Some(short) = name
            .strip_prefix('@')
            .and_then(|rest| rest.split_once('.'))
            .filter(|(ns, _)| METHOD_NAMESPACES.contains(ns))
            .map(|(_, short)| short.to_string())
        else {
            continue;
        };
        if NOT_METHOD_BUILTINS.contains(&name.as_str()) {
            continue;
        }
        if let Some(sig) = overloads
            .iter()
            .find(|sig| sig.params.first().map_or(false, |p| crate::checker::ty_matches(p, ty)))
        {
            out.push((short, sig.params[1..].to_vec(), sig.return_type.clone()));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

#[cfg(test)]
mod lib_sig_tests {
    use super::*;

    #[test]
    fn library_signatures_parse() {
        let (params, ret) = parse_sig("(string, map<string, string>, int) -> HttpResponse");
        assert_eq!(params, vec![Ty::String, Ty::Map(Box::new(Ty::String), Box::new(Ty::String)), Ty::Int]);
        assert_eq!(ret, Ty::Named("HttpResponse".to_string()));
        let (params, ret) = parse_sig("() -> array<string>");
        assert!(params.is_empty());
        assert_eq!(ret, Ty::Array(Box::new(Ty::String)));
    }

    #[test]
    fn every_library_function_has_a_valid_signature() {
        for (_, module, f) in spg_libs::all_functions() {
            let _ = parse_sig(f.sig);
            assert!(!f.doc.is_empty(), "{}.{} has no doc", module.name, f.name);
        }
    }
}
