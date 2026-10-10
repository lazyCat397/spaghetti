//! Builtin function signatures for the typechecker.
//!
//! Builtins are checked against this table instead of `Checker::functions`
//! so their params can use `Ty::Any`/`Ty::AnyArray` without polluting
//! `FunctionSig`, which mirrors the user's always-explicit `Param` syntax.
//!
//! A name can have more than one signature (overloads) — e.g. `@into.string`
//! accepts int, float, or bool, and array builtins accept any element
//! type via `Ty::AnyArray`. Call-site checking tries each signature in
//! order and takes the first exact arity+type match; this is a fixed,
//! finite table (not user-extensible), so linear scan is fine.
//!
//! Each builtin also carries a `BuiltinId`, the value the compiler and VM
//! use to identify it at the bytecode level — this is the single source
//! of truth both later stages key off of, so adding a builtin here is
//! the one place a signature has to be written by hand.

use crate::typed_ast::Ty;

/// identifies a builtin at the bytecode level; `spg-compiler` emits this
/// inside `OpCode::CallBuiltin` and `spg-vm` dispatches on it directly,
/// so the numbering only has to be stable within one compiled program
/// (there's no bytecode serialization across builds today).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum BuiltinId {
    Print,

    // ---- @json ----
    JsonStringify,
    JsonPretty,
    JsonGet,
    JsonHas,
    JsonType,
    JsonLen,
    JsonKeys,
    JsonValid,
    JsonSet,

    // ---- strings ----
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
    /// `@into.string(enumValue)` — chosen by the checker (no table entry)
    EnumToString,
    /// `@into.string(x)` for any other type: the same text `print` shows
    AnyToString,

    // ---- arrays ----
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
    /// (not including) `b`, as an array
    ArrayRange2,
    ArrayRange3,
    /// INTERNAL (no name in the table): emitted by the compiler for a
    /// `for .. in a..b step s` whose step is not a literal. Returns the step
    /// unchanged, or raises the zero-step runtime error.
    RangeStepCheck,

    // ---- @file ----
    FileRead,
    FileWrite,
    FileAppend,
    FileLines,
    FileExists,
    FileIsDir,
    FileList,
    FileMkdir,
    FileDelete,
    FileCopy,
    FileRename,
    FileJoin,
    FileDeleteAll,
    FileSize,
    FileModified,
    FileModifiedDate,
    FileReadBytes,
    FileWriteBytes,

    // ---- @sys ----
    SysArgs,
    SysExit,
    SysEnv,
    SysPlatform,
    SysCwd,
    SysReadLine,
    SysReadAll,

    // ---- more @str ----
    PadStart,
    PadStartFill,
    PadEnd,
    PadEndFill,
    StrLastIndexOf,
    TrimStart,
    TrimEnd,

    // ---- more @array ----
    ArrayInsert,
    ArrayRemove,
    ArrayClear,
    ArrayFill,
    ArrayConcat,
    ArraySumInt,
    ArraySumFloat,

    // ---- more @math ----
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

    // ---- maps (@map) ----
    MapHas,
    MapGet,
    MapRemove,
    MapLen,
    MapKeys,
    MapValues,
    MapKeysOf,

    // ---- math ----
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
    /// `@math.pi()` / `@math.e()`: the constant at the default precision (2 decimals)
    PiDefault,
    EDefault,

    // ---- type conversion (@into) ----
    IntToFloat,
    FloatToInt,
    ParseInt,
    ParseFloat,

    // ---- I/O ----
    ReadLine,
    PrintError,

    // ---- time ----
    TimeNow,
    TimeUnix,
    TimeElapsed,
    TimeSleep,
}

pub struct BuiltinSig {
    pub id: BuiltinId,
    pub params: Vec<Ty>,
    pub return_type: Ty,
}

/// every overload for a given builtin name, in the order call-site
/// checking should try them
impl BuiltinId {
    /// the `@array.*` builtins that take an element (or two arrays) whose type
    /// must agree with the array's own: the checker validates them itself
    /// (`check_array_call`) because the table only knows `Any` / `AnyArray`
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

    /// the `@map.*` builtins: their key/value types come from the map argument,
    /// so the checker types them itself (`check_map_call`) instead of by table
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

pub fn builtin_signatures() -> std::collections::HashMap<String, Vec<BuiltinSig>> {
    let mut m: std::collections::HashMap<String, Vec<BuiltinSig>> = std::collections::HashMap::new();

    let mut add = |name: &str, sig: BuiltinSig| {
        m.entry(name.to_string()).or_default().push(sig);
    };

    // ---- I/O ----
    add("print", BuiltinSig { id: BuiltinId::Print, params: vec![Ty::Any], return_type: Ty::Void });
    add(
        "printError",
        BuiltinSig { id: BuiltinId::PrintError, params: vec![Ty::Any], return_type: Ty::Void },
    );
    add("readLine", BuiltinSig { id: BuiltinId::ReadLine, params: vec![], return_type: Ty::String });

    // ---- strings ----
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
    // at this layer; the VM compares Values structurally at runtime)
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
    // `@into.string` takes int, float or bool
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

    // ---- arrays ----
    // `len` overloaded for array<T> alongside the string version above
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

    // ---- @file: the disk. Every I/O failure is a catchable runtime error ----
    let s = Ty::String;
    let strs = || Ty::Array(Box::new(Ty::String));

    // ---- @json: text <-> values. Reads work on the text by path; writes take any value ----
    add("@json.stringify", BuiltinSig { id: BuiltinId::JsonStringify, params: vec![Ty::Any], return_type: s.clone() });
    add("@json.pretty", BuiltinSig { id: BuiltinId::JsonPretty, params: vec![Ty::Any], return_type: s.clone() });
    add("@json.get", BuiltinSig { id: BuiltinId::JsonGet, params: vec![s.clone(), s.clone()], return_type: s.clone() });
    add("@json.has", BuiltinSig { id: BuiltinId::JsonHas, params: vec![s.clone(), s.clone()], return_type: Ty::Bool });
    add("@json.type", BuiltinSig { id: BuiltinId::JsonType, params: vec![s.clone(), s.clone()], return_type: s.clone() });
    add("@json.len", BuiltinSig { id: BuiltinId::JsonLen, params: vec![s.clone(), s.clone()], return_type: Ty::Int });
    add("@json.keys", BuiltinSig { id: BuiltinId::JsonKeys, params: vec![s.clone(), s.clone()], return_type: strs() });
    add("@json.valid", BuiltinSig { id: BuiltinId::JsonValid, params: vec![s.clone()], return_type: Ty::Bool });
    add("@json.set", BuiltinSig { id: BuiltinId::JsonSet, params: vec![s.clone(), s.clone(), Ty::Any], return_type: s.clone() });
    add("@file.read", BuiltinSig { id: BuiltinId::FileRead, params: vec![s.clone()], return_type: s.clone() });
    add("@file.write", BuiltinSig { id: BuiltinId::FileWrite, params: vec![s.clone(), s.clone()], return_type: Ty::Void });
    add("@file.append", BuiltinSig { id: BuiltinId::FileAppend, params: vec![s.clone(), s.clone()], return_type: Ty::Void });
    add("@file.lines", BuiltinSig { id: BuiltinId::FileLines, params: vec![s.clone()], return_type: strs() });
    add("@file.exists", BuiltinSig { id: BuiltinId::FileExists, params: vec![s.clone()], return_type: Ty::Bool });
    add("@file.isDir", BuiltinSig { id: BuiltinId::FileIsDir, params: vec![s.clone()], return_type: Ty::Bool });
    add("@file.list", BuiltinSig { id: BuiltinId::FileList, params: vec![s.clone()], return_type: strs() });
    add("@file.mkdir", BuiltinSig { id: BuiltinId::FileMkdir, params: vec![s.clone()], return_type: Ty::Void });
    add("@file.delete", BuiltinSig { id: BuiltinId::FileDelete, params: vec![s.clone()], return_type: Ty::Void });
    add("@file.copy", BuiltinSig { id: BuiltinId::FileCopy, params: vec![s.clone(), s.clone()], return_type: Ty::Void });
    add("@file.rename", BuiltinSig { id: BuiltinId::FileRename, params: vec![s.clone(), s.clone()], return_type: Ty::Void });
    add("@file.join", BuiltinSig { id: BuiltinId::FileJoin, params: vec![s.clone(), s.clone()], return_type: s.clone() });
    // recursive delete, kept under its own explicit name so `delete` stays the safe one
    add("@file.deleteAll", BuiltinSig { id: BuiltinId::FileDeleteAll, params: vec![s.clone()], return_type: Ty::Void });
    // size in bytes of a file (a directory is an error)
    add("@file.size", BuiltinSig { id: BuiltinId::FileSize, params: vec![s.clone()], return_type: Ty::Int });
    // last modification: milliseconds since 1970 (same unit as @time.unix) / ISO date in UTC (same format as @time.now)
    add("@file.modified", BuiltinSig { id: BuiltinId::FileModified, params: vec![s.clone()], return_type: Ty::Int });
    add("@file.modifiedDate", BuiltinSig { id: BuiltinId::FileModifiedDate, params: vec![s.clone()], return_type: s.clone() });
    // binary: a file as an array of bytes (ints 0..255)
    add("@file.readBytes", BuiltinSig { id: BuiltinId::FileReadBytes, params: vec![s.clone()], return_type: Ty::Array(Box::new(Ty::Int)) });
    add(
        "@file.writeBytes",
        BuiltinSig { id: BuiltinId::FileWriteBytes, params: vec![s.clone(), Ty::Array(Box::new(Ty::Int))], return_type: Ty::Void },
    );

    // ---- @sys: the process the program runs in ----
    add("@sys.args", BuiltinSig { id: BuiltinId::SysArgs, params: vec![], return_type: strs() });
    add("@sys.exit", BuiltinSig { id: BuiltinId::SysExit, params: vec![Ty::Int], return_type: Ty::Void });
    add("@sys.env", BuiltinSig { id: BuiltinId::SysEnv, params: vec![s.clone(), s.clone()], return_type: s.clone() });
    add("@sys.platform", BuiltinSig { id: BuiltinId::SysPlatform, params: vec![], return_type: s.clone() });
    add("@sys.cwd", BuiltinSig { id: BuiltinId::SysCwd, params: vec![], return_type: s.clone() });
    add("@sys.readLine", BuiltinSig { id: BuiltinId::SysReadLine, params: vec![], return_type: s.clone() });
    add("@sys.readAll", BuiltinSig { id: BuiltinId::SysReadAll, params: vec![], return_type: s.clone() });

    // ---- more @str ----
    // padStart/padEnd(s, width [, fill]): grow `s` to `width` characters with `fill` (default " "),
    // repeated and cut as needed; a shorter width or an empty fill leaves `s` as it is
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
    // index (in characters) of the LAST occurrence, or -1
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
    // checker validates the key and the fallback against the map's own types
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

    // ---- math ----
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
    // constants: `pi(n)` / `e(n)` give n decimal places (rounded); without an
    // argument the default precision is 2 decimals (3.14 / 2.72). The `(int)`
    // form is registered first so a wrong argument reports against it.
    add("@math.pi", BuiltinSig { id: BuiltinId::PiDecimals, params: vec![Ty::Int], return_type: Ty::Float });
    add("@math.pi", BuiltinSig { id: BuiltinId::PiDefault, params: vec![], return_type: Ty::Float });
    add("@math.e", BuiltinSig { id: BuiltinId::EDecimals, params: vec![Ty::Int], return_type: Ty::Float });
    add("@math.e", BuiltinSig { id: BuiltinId::EDefault, params: vec![], return_type: Ty::Float });

    // trigonometry (radians), exponentials and logarithms: float -> float; the
    // domain errors follow IEEE (`log(0.0)` is -inf, `asin(2.0)` is NaN), same as sqrt
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
    // -1, 0 or 1 (a NaN gives 0)
    add("@math.sign", BuiltinSig { id: BuiltinId::SignInt, params: vec![Ty::Int], return_type: Ty::Int });
    add("@math.sign", BuiltinSig { id: BuiltinId::SignFloat, params: vec![Ty::Float], return_type: Ty::Int });
    // clamp(x, low, high): x limited to [low, high]; low > high is a runtime error
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
    // `@into.int`: float → int truncates toward zero; string → int parses
    // (and fails at runtime with InvalidNumberFormat)
    add("@into.int", BuiltinSig { id: BuiltinId::FloatToInt, params: vec![Ty::Float], return_type: Ty::Int });
    add("@into.int", BuiltinSig { id: BuiltinId::ParseInt, params: vec![Ty::String], return_type: Ty::Int });
    add("@into.float", BuiltinSig { id: BuiltinId::IntToFloat, params: vec![Ty::Int], return_type: Ty::Float });
    add("@into.float", BuiltinSig { id: BuiltinId::ParseFloat, params: vec![Ty::String], return_type: Ty::Float });

    // ---- time ----
    add("@time.now", BuiltinSig { id: BuiltinId::TimeNow, params: vec![], return_type: Ty::String });
    add("@time.unix", BuiltinSig { id: BuiltinId::TimeUnix, params: vec![], return_type: Ty::Int });
    add("@time.elapsed", BuiltinSig { id: BuiltinId::TimeElapsed, params: vec![], return_type: Ty::String });
    add("@time.sleep", BuiltinSig { id: BuiltinId::TimeSleep, params: vec![Ty::Int], return_type: Ty::Void });

    // ---- the one global shortcut: `len` (@str.len / @array.len) ----
    add("len", BuiltinSig { id: BuiltinId::Len, params: vec![Ty::String], return_type: Ty::Int });
    add("len", BuiltinSig { id: BuiltinId::ArrayLenBuiltin, params: vec![Ty::AnyArray], return_type: Ty::Int });

    m
}

/// Old flat builtin name → its `@namespace` replacement, so a program written
/// before namespaces gets "use '@math.sqrt'" instead of a bare "undefined
/// function". `len` is intentionally absent: it stays global.
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
        // shared between string and array: which one depends on the argument
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

/// The previous namespace spelling (`math.sqrt`, `string.len`, `list.push`,
/// `transform.toInt`, `time.run`, ...) → the current `@` name. Only returns a
/// name that exists in the builtin table, so the hint is always correct.
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
/// `@ns.fn(x, ...)` whenever the first parameter of `@ns.fn` takes `x`
pub const METHOD_NAMESPACES: [&str; 5] = ["str", "array", "map", "math", "into"];

/// builtins that never act as methods even though their first parameter is an
/// int or float: constants, a random source, and the two builders whose first
/// argument is a count or a bound, not a receiver
pub const NOT_METHOD_BUILTINS: [&str; 6] = [
    "@math.pi",
    "@math.e",
    "@math.tau",
    "@math.random",
    "@array.range",
    "@array.fill",
];

/// the namespace (of `METHOD_NAMESPACES`) whose function `method` takes a value
/// of type `ty` as its first argument: `x.abs()` for an int is `@math.abs(x)`.
/// `None` when no such function exists. A namespace listed earlier wins, and
/// the table has no name shared by two namespaces for the same receiver type.
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
/// `(name, parameters after the receiver, return type)`. Each name is taken
/// from the first overload that takes `ty` first, so `x.min(y)` on an int
/// shows the int overload. Used by the LSP to offer the methods of numbers.
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
