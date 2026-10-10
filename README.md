# Spaghetti — CLI and VSCode extension

## Install (Windows)

1. Put this folder anywhere (for example `C:\spaghetti-prot`).
2. Double-click **`build.bat`** (or run `build.bat` in a terminal opened in this folder).
   It is an interactive installer: it checks Rust and VSCode, asks where to install
   (default `C:\spaghetti`), whether to add it to the PATH, install the VSCode extension,
   copy the examples and run a self-test, shows the plan, and only then builds and installs.
   If the chosen folder already exists and is **not** a Spaghetti install, it asks whether to
   replace it (you must type `YES`) or rename the existing one out of the way. An existing
   Spaghetti install is offered an update instead. If the build fails, nothing is changed.
3. Open a NEW terminal (and reload VSCode) so they pick up the PATH.

Options: `build.bat -Yes` (accept every default, ask nothing; refuses to touch a folder it does
not recognise), `build.bat -InstallDir D:\spg`, `build.bat -NoBuild` (install the existing
`target\release\spg.exe`).

No `code` command on the PATH? In VSCode: Extensions → `...` → *Install from VSIX* →
`dist\spaghetti-0.19.0.vsix`.

## Use

```
spg file.spg            run a program
spg run file.spg        same thing
spg check file.spg      type-check only, don't run
spg lsp                 language server (VSCode starts it for you)
spg --version | --help
```

`run` options: `--time`, `--dump-bytecode`. Exit codes: 0 ok · 1 error in the program or
its source · 2 bad usage.

## The executable is standalone

Zero external crates (Rust std only) and a statically linked C runtime
(`.cargo/config.toml`): `spg.exe` runs on any Windows without installing anything.
Rust is only needed to *build* it.

## Uninstall

`powershell -ExecutionPolicy Bypass -File uninstall.ps1` shows what it found and asks before removing
`spg.exe`, the examples, the PATH entry, `SPG_HOME` and the VSCode extension. Your own files in the
install folder are never deleted.

## Layout

```
crates/            engine (lexer, parser, modules, typecheck, compiler, vm) + spg-lsp + spg-cli
editors/vscode/    extension source code
dist/              ready-made spaghetti-0.19.0.vsix
tools/installer-tests/  tests for install.ps1 / uninstall.ps1 (PowerShell 7)
```
