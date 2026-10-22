# Spaghetti

A small statically typed language with its own minimally optimized bytecode VM, written in Rust.
Files use the `.spg` extension.

## Layout

- `crates/` the compiler, VM, type checker, module loader, formatter, CLI and language server
- `editors/vscode/` the VS Code extension (source; packaged by the release workflow)
- `installer/` the Windows installer script and the Linux install script
- `examples/` short learning samples, numbered in the order they build on each other
- `tests/` the regression suite (`tests/run.sh`), with programs grouped by feature
- `tools/` tests for the extension and the grammar

## Build

    cargo build --release
    ./target/release/spg examples/hello.spg

## Test

    cargo test --workspace
    bash tests/run.sh

## Release

Pushing a tag `vX.Y.Z` runs `.github/workflows/release.yml`: it runs the tests, packages the
extension, builds the Windows installer and the Linux package, and publishes them.

## License

Licensed under either of the [Apache License, Version 2.0](LICENSE-APACHE) or the [MIT license](LICENSE-MIT), at your option.
