# Spaghetti

Spaghetti is a small, statically typed programming language with its own bytecode VM and a
Cranelift JIT, written in Rust. Source files use the `.spg` extension.

```
let fruits: array<string> = ["apple", "pear"]
fruits.push("plum")
print(@array.len(fruits), fruits[0])
```

It aims to be easy to read and to write: blocks use braces, types are checked before the
program runs, and the standard tools (formatter, language server, VS Code extension) come in
the same package.

Highlights:

- static types: `int`, `float`, `bool`, `string`, `array<T>`, `map<K, V>`, structs, enums and optionals (`T?`)
- unions (`string or int`), checked at compile time
- `when` for matching, `try` / `catch` for errors, `async` tasks with `.wait()`
- pipes (`|>`), lambdas, default parameters, string interpolation (`"${expr}"`)
- libraries installed with `load #name` (`http`, `json`, `ui`, `sqlite`, `regex`, ...)

Examples live in `examples/`, numbered in the order they build on each other.
Start with `examples/01_hello.spg`.

## Layout

- `crates/` the compiler, VM, JIT, type checker, module loader, formatter, CLI and language server
- `editors/vscode/` the VS Code extension (source; packaged by the release workflow)
- `installer/` the Windows installer script and the Linux install script
- `examples/` short learning samples, numbered (`01_hello.spg` ...)
- `examples/ui/` window demos for the `ui` library (they need a display, so the test suite does not run them)
- `examples/assets/` fonts and images used by those demos
- `examples/lib/` small modules used by `12_modules.spg`
- `stdlib/` standard library modules written in Spaghetti
- `tests/` the regression suite (`tests/run.sh`), with programs grouped by feature
- `tools/` tests for the extension and the grammar

## Build

    cargo build --release
    ./target/release/spg examples/01_hello.spg

## Test

    cargo test --workspace
    bash tests/run.sh

## Release

Pushing a tag `vX.Y.Z` runs `.github/workflows/release.yml`: it runs the tests, packages the
extension, builds the Windows installer and the Linux package, and publishes them.

## License

Licensed under either of the [Apache License, Version 2.0](LICENSE-APACHE) or the [MIT license](LICENSE-MIT), at your option.
