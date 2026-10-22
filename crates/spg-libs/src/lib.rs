//! spg-libs: the libraries that ship with the engine but are not part of the language.
//!
//! ONE FILE PER LIBRARY. a library is a `NativeModule`: its name, the structs its calls return,
//! and its functions, each one with its signature, its docs and the Rust code that runs it.
//! the language never learns what is inside: the lexer asks `module(name)` whether `install
//! #name` is a native library, the typechecker builds the signatures from `sig`, the
//! VM calls `call`, the editor reads `doc`. to add a library: write `src/<name>.rs` and add one
//! line to `MODULES` below. that's the whole recipe.
//!
//! this crate depends on no other spg crate, so a library only sees plain Rust values
//! (`LibValue`); the VM converts to and from its own values at the boundary.

mod chars;
mod crypto;
mod csvtext;
mod file;
mod http;
mod json;
mod json5lib;
mod cmd;
mod ps;
mod regexp;
mod socket;
mod sqlite;
mod testing;
mod time;
mod tomlcfg;
mod util;

/// a value crossing the boundary between the VM and a library
#[derive(Debug, Clone, PartialEq)]
pub enum LibValue {
    Void,
    /// the program's `nil`
    Nil,
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    Array(Vec<LibValue>),
    /// key/value pairs in insertion order
    Map(Vec<(LibValue, LibValue)>),
    /// a struct: the program's own (a library gets one when it is passed in) or one the module
    /// declares in `structs` (a library hands one back). fields by name, so a library that
    /// reads any value (`@json.stringify`) sees the names, and the VM can rebuild the struct
    Struct { name: String, fields: Vec<(String, LibValue)> },
    /// an enum value, as the text the program would print for it
    Enum(String),
    /// a value a library can not look inside: "function", "task", or "deep" (nested too deeply,
    /// does it contain itself?)
    Opaque(&'static str),
}

impl LibValue {
    /// a struct value with its fields in declaration order
    pub fn record(name: &str, fields: Vec<(&str, LibValue)>) -> LibValue {
        LibValue::Struct { name: name.to_string(), fields: fields.into_iter().map(|(n, v)| (n.to_string(), v)).collect() }
    }
}

/// what the VM tells a call about where it happens
pub struct CallCtx<'a> {
    /// the script file that made the call (relative paths are relative to its folder)
    pub caller_file: Option<&'a str>,
    /// the line of the call in that file (0 when unknown)
    pub caller_line: u32,
}

/// a failure to do the job. becomes a runtime error, which the program can `catch`.
#[derive(Debug)]
pub struct LibError {
    /// `"ps.run"`, `"http.get"`: where it came from
    pub function: &'static str,
    pub reason: String,
    /// the file or folder involved, for IO errors: shown as `function("path"): reason`
    pub path: Option<String>,
}

impl LibError {
    pub fn new(function: &'static str, reason: impl Into<String>) -> LibError {
        LibError { function, reason: reason.into(), path: None }
    }

    /// an error about a path (`@file.read("a.txt"): no such file or directory`)
    pub fn io(function: &'static str, path: &str, reason: impl Into<String>) -> LibError {
        LibError { function, reason: reason.into(), path: Some(path.to_string()) }
    }
}

/// how a library shows up in a program
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// always there, written with `@` (`@file.read`): part of what the engine gives for free
    Core,
    /// needs `install #name`, written without `@` (`ps.run`)
    Install,
}

pub type LibFn = fn(&CallCtx, Vec<LibValue>) -> Result<LibValue, LibError>;

/// one overload of one function. a name with several shapes (`run(cmd)`, `run(cmd, timeout)`)
/// is several `NativeFn` with the same `name`, tried in order.
pub struct NativeFn {
    pub name: &'static str,
    /// `"(string, int) -> PsResult"`. types: int, float, bool, string, void, array<T>, map<K, V>,
    /// or the name of one of the module's structs
    pub sig: &'static str,
    pub doc: &'static str,
    pub call: LibFn,
}

pub struct NativeModule {
    pub kind: Kind,
    /// the namespace, and the name in `install #name`
    pub name: &'static str,
    /// what the library is for, shown by the editor
    pub doc: &'static str,
    /// spg source of the structs the calls return. the engine puts them in any program that
    /// installs the library, and the editor reads the same text for completions
    pub structs: &'static [&'static str],
    pub functions: &'static [NativeFn],
}

/// every native library. adding one = one line here.
pub static MODULES: [&NativeModule; 15] = [
    &ps::MODULE,
    &http::MODULE,
    &file::MODULE,
    &json::MODULE,
    &time::MODULE,
    &regexp::MODULE,
    &chars::MODULE,
    &crypto::MODULE,
    &csvtext::MODULE,
    &testing::MODULE,
    &tomlcfg::MODULE,
    &sqlite::MODULE,
    &socket::MODULE,
    &cmd::MODULE,
    &json5lib::MODULE,
];

pub fn modules() -> &'static [&'static NativeModule] {
    &MODULES
}

/// the native library called `name`, if there is one
pub fn module(name: &str) -> Option<&'static NativeModule> {
    MODULES.iter().copied().find(|m| m.name == name)
}

/// every function of every library, each with its global index (what the bytecode stores)
pub fn all_functions() -> Vec<(u16, &'static NativeModule, &'static NativeFn)> {
    let mut out = Vec::new();
    for m in MODULES.iter().copied() {
        for f in m.functions {
            out.push((out.len() as u16, m, f));
        }
    }
    out
}

/// the function behind a global index (no allocation: this runs on every library call)
pub fn function(index: u16) -> Option<(&'static NativeModule, &'static NativeFn)> {
    let mut left = index as usize;
    for m in MODULES.iter().copied() {
        if left < m.functions.len() {
            return Some((m, &m.functions[left]));
        }
        left -= m.functions.len();
    }
    None
}

/// the global index of the first overload of `module.function`
pub fn index_of(module: &str, function: &str) -> Option<u16> {
    all_functions().into_iter().find(|(_, m, f)| m.name == module && f.name == function).map(|(i, _, _)| i)
}

/// `#> safe-party`: set once by the CLI from the entry file's header
pub use ps::set_safe_party;

/// `Vm::new` calls this so `@time.elapsed` counts from when the program started
pub use time::mark_program_start;

/// `install #test`: the checks of the running program, for the CLI to report at the end
pub use testing::{take_report as take_test_report, TestFailure, TestReport};
