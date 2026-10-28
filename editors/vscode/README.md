# Spaghetti for VSCode

Language support for **Spaghetti** (`.spg`).

- **Syntax highlighting**: keywords, types, builtins, structs, strings with escapes
- **Live errors**: the same lexer, parser, module resolver and typechecker as `spg`, right in the editor — including unsaved files and imports between files
- **Completion**: keywords, types, every builtin (with its signature) and the names you declared in the file. Namespaces open as you type: typing `@` lists `@str`, `@array`, `@map`, `@into`, `@math`, `@time`, `@file` and `@sys`; typing `.` after one of them lists its members right away, and picking a namespace from the list inserts `@math.` (dot included) and opens its members. After a struct value's `.` (`p.`, `me.`, `a.pos.`) its fields and methods are suggested, and after a string, array or map (`s.`, `xs.`, `m.`) its native methods (`xs.push(1)`); `throw` is a keyword and `e` inside `catch (e)` is a string (an enum value offers `value`), and inside `Point { ` the fields you have not written yet; after any other `.` (a number, a call) nothing is suggested
- **Formatting**: `Format Document` (`Shift+Alt+F`) and format on save, from the same formatter as `spg fmt`. Only spacing and indentation change; comments, strings and tokens are kept, and a file with a syntax error is left alone. To turn format on save off for `.spg` files: `"[spg]": { "editor.formatOnSave": false }`
- **Highlighting** for the whole language, including string interpolation (`"hi ${name}"`, with code inside the braces colored as code), optional types and `none` (`int?`, `none`, `??`), and function values (`fn(x: int) -> int { ... }`).
- **Snippets**: `fn`, `let`, `const`, `if`, `ife`, `ifelif` (if/elif/else), `when` (match), `while`, `for`, `struct`, `try`, `import`, `exportfn`, `reexport`, `print`
- **Run** (`▶` at the top of the editor, or `Ctrl+F5`) and **Type-check** (command palette)

## Requirement

The `spg` executable on the PATH (the project installer does this), or its path in the
`spg.path` setting.

## Settings

| Key | Default | What it does |
|---|---|---|
| `spg.path` | *(empty)* | Path to `spg`. Empty = use the PATH |
| `spg.server.enable` | `true` | Turns live errors, completion and formatting on/off |
| `spg.trace.server` | `off` | `messages` logs every message to the output channel |

## Commands

`Spaghetti: Run file` · `Spaghetti: Type-check` · `Spaghetti: Restart language server` · `Spaghetti: Show server log`

## File directives (`#>`)

Lines that start with `#>` at the very top of the entry file change how the whole program runs:

| Directive | What it does |
|---|---|
| `#> strict` | int overflow, a name hiding another one, and code after `return`/`break`/`continue`/`throw` become errors |
| `#> timeout <ms>` | stops the program after `<ms>` milliseconds (exit code 124); it also cuts `@time.sleep` and `@sys.readLine` |
| `#> friendly-terminal` | error messages in a friendlier tone |

The extension highlights these lines, completes the directive names after `#>` (skipping the ones already used), and the editor errors follow `#> strict`. A misspelled, repeated or misplaced directive is an error with a "did you mean" hint.
