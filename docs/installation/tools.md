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
spg fmt --min hello.spg    // format in the compact layout (see below)
```

`check` reports the same source errors that a run would report, so it is a quick way to test a file without running it. `fmt` rewrites the file with the standard layout.

### Compact layouts

`spg fmt --min` writes the file in a compact layout: no indentation, no blank lines, no spaces that are not needed, and a `;` where a line used to end. The comments stay. `spg fmt --min-nc` does the same and removes the comments too. Before writing, the formatter checks that the tokens, the comments and the parsed program are the same as before. The standard `spg fmt` layout does not change: running it on a compact file expands it again.

```javascript
let ex = 2
if (ex == 2) {
   print(true)
} else {
   print(false)
}
```

becomes:

```javascript
let ex=2;if(ex==2){print(true)}else{print(false)}
```

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

The default build file is about 13 MB, because it carries the whole runtime, with the JIT and every native library. `spg build --lean` makes a smaller file, about 3 MB, from `spg-lean`:

```javascript
spg build --lean hello.spg -o hello
```

The lean runtime has no JIT, so loops run several times slower than with the default build. It has only the libraries that need no extra code: `ps`, `file`, `json`, `json5`, `time`, `char`, `cmd`, `net`, `test`. A program that opens a window (`attach #ui`) gets `spgw-lean`, about 4 MB, which also has the window library. A program that attaches `#http`, `#regex`, `#crypto`, `#csv`, `#toml` or `#sqlite` is refused by `--lean`, with a message that names the library, and it can be built without the flag. Use `--lean` for small programs where the size matters more than the speed.

### Renaming names: `--enc`

`spg build --enc` makes the program hard to read. The names of its functions, variables, parameters and loop variables become random ones, and its comments go. The program runs the same way:

```javascript
spg build --enc hello.spg -o hello
```

The names of struct and enum types, of fields and variants, of methods, and of anything after a `.` stay as they are, because they show up when a value is printed and because libraries use them. So do the names of the native libraries and the names of the standard library, which stays readable. String literals stay readable too. The new names are the same every time the same sources are built.

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
