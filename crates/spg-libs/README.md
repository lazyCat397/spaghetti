# spg-libs

The libraries that ship with the engine but are not part of the language: `ps`, `http`, `file`, `json`, `time`.
**One file per library.** The lexer, the typechecker, the VM and the editor never learn what is inside a library; they all read the registry in `src/lib.rs`.

## Two kinds

| `kind`          | How a program uses it                                  | Example         |
|-----------------|--------------------------------------------------------|-----------------|
| `Kind::Core`    | always there, written with `@`                         | `@file.read(p)` |
| `Kind::Install` | `attach #name` first, written **without** `@` | `ps.run(cmd)`   |

The rule: `@` means "the engine gives you this for free". No `@` means "you installed it".
Use `Core` only for things almost every script needs. Everything heavy or optional is `Install`.

## Make a new library (3 steps)

**1. Create `src/hello.rs`:**

```rust
use crate::{CallCtx, Kind, LibError, LibValue, NativeFn, NativeModule};

pub static MODULE: NativeModule = NativeModule {
    kind: Kind::Install,
    name: "hello",                 // the namespace: hello.greet(...)
    doc: "a tiny example library", // shown by the editor
    structs: &[],                  // spg source of structs your functions hand back (see below)
    functions: &[
        NativeFn { name: "greet", sig: "(string) -> string", doc: "says hello to a name", call: greet },
    ],
};

fn greet(_: &CallCtx, args: Vec<LibValue>) -> Result<LibValue, LibError> {
    match args.first() {
        Some(LibValue::Str(name)) => Ok(LibValue::Str(format!("hello, {name}!"))),
        _ => Err(LibError::new("hello.greet", "expected a name")),
    }
}
```

**2. Register it in `src/lib.rs`:** add `mod hello;` and add `&hello::MODULE` to `MODULES` (and bump the array length).

**3. Build.** That's it. `attach #hello` then `hello.greet("world")` works, with type checking, autocomplete, hover docs and errors.

## The details

**Signatures** are text: `"(string, int) -> HttpResponse"`. Allowed types: `int`, `float`, `bool`, `string`, `void`, `any`, `array<T>`, `map<K, V>`, or the name of one of your structs.
A name with several shapes is several `NativeFn` with the same `name` (see `ps.run` and `http.get`); the typechecker picks the one that fits.

**Values** cross the boundary as `LibValue` (`Int`, `Float`, `Bool`, `Str`, `Array`, `Map`, ...). The VM converts to and from its own values for you. The typechecker already checked the argument types, so reading them is just a `match`.

**Structs** your functions hand back are declared as spg source in `structs`, and built with `LibValue::record`. Field names must match the declaration:

```rust
structs: &["struct Point {\n    x: int,\n    y: int\n}\n"],
// and in a function with sig "(int) -> Point":
Ok(LibValue::record("Point", vec![("x", LibValue::Int(1)), ("y", LibValue::Int(2))]))
```

**Errors:** `LibError::new("hello.greet", "what went wrong")`. The program can `catch` it. For a file or folder problem use `LibError::io(function, path, reason)`; it prints as `function("path"): reason`.

**Context:** `CallCtx::caller_file` is the script that made the call (for resolving relative paths).

## Things to know

- **Pick a name that is free.** Not one of the engine's own: `str`, `array`, `map`, `into`, `math`, `sys`, `task`.
- **A library only sees plain values.** It can't touch the VM. The one thing a library can't do alone is give the baton away to other tasks (`@time.sleep`), so the VM runs that call itself and `time.rs` only declares it. Keep new libraries to plain work.
- **Each call copies its values.** Fine for IO, files, JSON, the web. Don't build hot numeric loops on top of a library: those builtins (`@math`, strings, arrays) stay in the engine, where the JIT knows them.
- **Test it:** `cargo test -p spg-libs` checks that every function has a valid signature and a doc. Add a small script to `tests/ok/` and an entry in `tests/run.sh` like the other libraries have, then run `bash tests/run.sh`.
