# Tools: the command line, the JIT and the editor

This page covers the `spg` command in more detail, the backend that runs your programs, the `build` command for standalone programs, and the VSCode extension.

## The `spg` command

`spg` runs, checks, formats and builds Spaghetti programs. The quick reference is in [getting-started.md](../getting-started.md#quick-reference). This section explains the details.

### Commands

```javascript
spg run hello.spg          // run a program (same as: spg hello.spg)
spg check hello.spg        // check types, do not run
spg fmt hello.spg          // format the file in place
spg build hello.spg -o app // build a standalone program
spg lsp                    // start the language server (used by editors)
```

`check` reports the same source errors that a run would report, so it is a quick way to test a file without running it. `fmt` rewrites the file with the standard layout.

### Flags

| Flag | What it does |
|---|---|
| `--version` | Prints the version and the build id, for example `spg 1.0.0 (a1b2c3d)`. |
| `--time` | Prints only the time the program took to run inside the VM. |
| `--dump-bytecode` | Prints the bytecode that the compiler produces. Useful when you work on the language itself. |
| `--no-opt` | Turns off the bytecode optimizer. |
| `--no-jit` | Runs the program in the VM only, without the JIT. |
| `--safe-party` | Runs with the safe-party limits on, even when the file does not say `#> safe-party`. See [overview.md](../libraries/overview.md#safe-mode). |

`--time` is useful for measuring a change. The time it prints does not include starting the program, so for a short program it shows only a fraction of a millisecond.

```javascript
spg --time hello.spg
```

The output also includes a line that says how many functions the JIT compiled, as `native: N function(s) compiled`. See the next section.

## The JIT

Spaghetti runs programs with a JIT (just-in-time compiler), built on Cranelift. The JIT turns hot functions into native machine code while the program runs. It is the default, so you do not need to do anything to use it.

The JIT covers integers, floats, booleans, strings, arrays, structs and output. When a function does something the JIT does not cover, that function runs in the VM. The same goes for operations that need a check the JIT cannot make, such as integer overflow, division by zero, or a call that is too deep. In those cases the VM takes over, so the program behaves the same either way.

To run everything in the VM, add `--no-jit`:

```javascript
spg --no-jit hello.spg
```

You rarely need this flag. It is useful when you want to rule out the JIT as the cause of a problem, or when you compare speed.

## Building a standalone program

`spg build` produces one executable file. The file contains the Spaghetti runtime and your source code, so the person who runs it does not need Spaghetti installed:

```javascript
spg build hello.spg -o hello
```

The built program runs with the same speed as `spg run`. Its source is embedded in the file, so anyone who has the built file can read the code through the bytes in it. Do not build a program if its source needs to stay private.

## The VSCode extension

The Windows installer and the Linux script install the extension when VSCode is available. You can also install it yourself from the `spaghetti.vsix` file in the release.

The extension gives you:

- **Running:** press **Ctrl+F5** to run the open `.spg` file.
- **Autocomplete:** suggests functions, namespaces such as `@array`, members of structs and enums, and the names that other files make public.
- **Install suggestions:** completes `attach #ui` and the other libraries.
- **Path checks:** an invalid path in an `use`, `attach`, font or image is flagged while you type.
- **Snippets and colors:** common patterns for `fn`, `struct`, `enum`, `try` and loops, plus syntax colors.

The language server runs through `spg lsp`, so the editor always uses the same checker as the command line.

Known limitation: hover information does not work yet. Autocomplete and errors do.

## Checking your setup

If a command is not found after installing, check the PATH:

- On Windows, open a new terminal after installing. The installer adds `%USERPROFILE%\Spaghetti` to your user PATH when you keep the first option checked.
- On Linux, make sure `~/.local/bin` is on your PATH. See [getting-started.md](../getting-started.md#linux).

Run `spg --help` to see every command and flag that your version supports.
