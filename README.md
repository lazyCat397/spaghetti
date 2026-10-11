# Spaghetti

A small statically typed language with its own bytecode VM, written in Rust.
Files use the `.spg` extension.

## Layout

- `crates/` the compiler, VM, type checker, module loader, formatter, CLI and language server
- `editors/vscode/` the VS Code extension (source; packaged by the release workflow)
- `installer/` the Windows installer script and the Linux install script
- `examples/` sample programs; `examples/multi/` is the regression suite
- `tools/` tests for the extension and the grammar

## Build

    cargo build --release
    ./target/release/spg examples/hello.spg

## Test

    cargo test --workspace
    bash examples/multi/run.sh

## Release

Pushing a tag `vX.Y.Z` runs `.github/workflows/release.yml`: it runs the tests, packages the
extension, builds the Windows installer and the Linux package, and publishes them.
