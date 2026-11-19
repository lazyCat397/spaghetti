# Getting started

This page shows how to install Spaghetti, run your first program, and find the rest of the documentation.

## Install

Download the file for your system from the releases page:

- Windows: `spaghetti-1.0.0-windows-setup.exe`
- Linux: `spaghetti-1.0.0-linux.tar.gz`

### Windows

1. Run `spaghetti-1.0.0-windows-setup.exe`.
2. Windows SmartScreen may stop the file, because it is not signed yet. Click **More info**, then **Run anyway**.
3. Choose the language of the installer: English or Brazilian Portuguese.
4. The installer opens. You do not need Inno Setup on your computer, because the installer already includes what it needs.
5. Answer the options. The defaults are good for most people:

   - **Add spg to PATH (recommended):** lets you run `spg` from any terminal.
   - **Associate .spg files with Spaghetti:** opens `.spg` files with `spg` when you double-click them.
   - **Install the VSCode extension (if VSCode is installed):** adds editor support. It is skipped when VSCode is not found.
   - **Create a desktop shortcut:** off by default.

6. Click install and wait for it to finish.

<!-- screenshot: the installer options page -->

Spaghetti goes to your user folder, `%USERPROFILE%\Spaghetti` (for example, `C:\Users\<you>\Spaghetti`). No administrator permission is needed. The `stdlib` and `examples` folders are installed next to `spg.exe`.

After installing, open VSCode, open a `.spg` file, and press **Ctrl+F5** to run it.

To uninstall, open the Spaghetti folder and run `unins000.exe`. You can also remove Spaghetti from **Settings → Apps**.

### Linux

1. Extract the archive: `tar -xzf spaghetti-1.0.0-linux.tar.gz`.
2. Open the folder it created, and run the install script in it: `bash install-linux.sh`.

The script copies `spg` and the `examples` folder to `~/.local/bin`, and installs the VSCode extension if the `code` command is available.

If `~/.local/bin` is not on your PATH, the script says so. Add this line to `~/.bashrc` or `~/.zshrc`, then open a new terminal:

```javascript
export PATH="$HOME/.local/bin:$PATH"
```

To uninstall, remove the `spg` file and the `examples` folder from `~/.local/bin`.

### Check the install

Run this in a terminal:

```javascript
spg --version
```

It prints the version:

```
spg 1.0.0 (a1b2c3d)
```

The part in brackets is the build id: the short git hash of the source the binary was built from. A `-dirty` suffix means it was built with local changes. It tells you which copy of `spg` you are running.

## Your first program

Create a file named `hello.spg` with this line:

```javascript
print("Hello, Spaghetti!")
```

Run it:

```
spg hello.spg
```

```
Hello, Spaghetti!
```

`spg hello.spg` is a shortcut for `spg run hello.spg`. Both do the same thing.

You can also open the file in VSCode and press **Ctrl+F5** to run it.

### A longer tour

This program uses the main parts of the language. Copy it into a file named `tour.spg` and run it:

```javascript
// variables: let can change, const cannot
let score = 10
const name: string = "Spaghetti"
score += 5
print(name, score)

// functions take typed parameters and return a typed value
fn double(n: int) -> int {
    return n * 2
}
print(double(score))

// if, with elif and else
if (score > 10) {
    print("big")
} elif (score == 10) {
    print("exact")
} else {
    print("small")
}

// a loop over a range, where the end is not included
let total = 0
for (n in 1..4) {
    total += n
}
print("total:", total)

// arrays and maps
let fruits: array<string> = ["apple", "pear"]
fruits.push("plum")
print(fruits.len(), fruits[2])

let stock: map<string, int> = ["apple": 3]
stock["pear"] = 5
print(stock)

// structs
struct Point { x: int, y: int }
let p = Point { x: 1, y: 2 }
print(p.x + p.y)

// a union: the value can be one of several types
let id: int or string = 7
print(type(id))

// text with interpolation
print("Hello, ${name}! Score: ${score}")
```

Its output:

```
Spaghetti 15
30
big
total: 6
3 plum
[apple: 3, pear: 5]
3
int
Hello, Spaghetti! Score: 15
```

<!-- gif: running tour.spg in VSCode with Ctrl+F5 -->

### Reading the errors

Spaghetti checks your program before it runs. If something is wrong, it tells you where:

```javascript
const c: int = 1
c = 2
// type error in c.spg at line 2, column 3: cannot assign to 'c' — it's declared as const
```

```javascript
let n = 1
print(n + "a")
// type error in t.spg at line 2, column 9: operator Add not supported for int and string
```

The message names the file, the line and the column. Read it from the top, and fix one error at a time. Each error includes what went wrong.

For a friendlier tone, add `#> friendly-terminal` as the first line of a file. See [errors-and-debugging.md](standard-lib/errors-and-debugging.md) for details.

## Examples

The installer copies the `examples` folder. Each file shows one part of the language, and the numbers give a good reading order:

- `01_hello.spg`: the smallest program.
- `02_variables.spg` to `05_control_flow.spg`: variables, types, functions and control flow.
- `06_collections.spg` to `09_errors.spg`: arrays, maps, structs, enums and errors.
- `10_pipes.spg` to `12_modules.spg`: pipes, the standard library and modules.
- `13_loops_and_operators.spg`: loops and operators.
- `ui/`: examples of the `ui` library, for windows, shapes, text and images.
- `ui/clock/` and `ui/shooter/`: two complete programs, an analog clock and a shoot 'em up. See [ui.md](libraries/ui.md#full-demos).

## Where to go next

Each part of the language has its own page:

- [collections.md](about/collections.md) explains arrays and maps.
- [structs-and-enums.md](about/structs-and-enums.md) covers structs, methods and enums.
- [optionals.md](about/optionals.md) covers `T?`, `none` and `??`.
- [functions-and-lambdas.md](about/functions-and-lambdas.md) covers functions as values and closures.
- [modules.md](about/modules.md) explains splitting a program into files.
- [text-and-numbers.md](standard-lib/text-and-numbers.md) covers strings, numbers and conversions.
- [numbers.md](standard-lib/numbers.md) covers the integer and float types, from `i8` to `u64` and `f32`, with overflow and conversions.
- [maps-and-lists.md](standard-lib/maps-and-lists.md) is the reference for `@array` and `@map`.
- [data-and-files.md](standard-lib/data-and-files.md) covers files and JSON.
- [system-and-time.md](standard-lib/system-and-time.md) covers arguments, input, exit codes and time.
- [whats-new.md](whats-new.md) lists the latest changes to the language, with an example for each.
- [errors-and-debugging.md](standard-lib/errors-and-debugging.md) covers `throw`, `try`, stack traces and directives.
- [tools.md](installation/tools.md) covers the command line, the editor extension and the JIT.
- [overview.md](libraries/overview.md) covers `attach #name` and the libraries that come with Spaghetti, such as `ps`.
- [ui.md](libraries/ui.md) covers the `ui` library: windows, shapes, text, images and input.

## Quick reference

| Command | What it does |
|---|---|
| `spg file.spg` | Runs a program. |
| `spg run file.spg` | Same as above. |
| `spg check file.spg` | Checks types without running. |
| `spg fmt file.spg` | Formats the file. |
| `spg build file.spg -o name` | Builds a standalone executable. |
| `spg --version` | Prints the version and the build id. |
| `spg --help` | Prints the help. |

Exit codes: `0` means success, `1` means an error in the program or its source, and `2` means wrong use of the command.
