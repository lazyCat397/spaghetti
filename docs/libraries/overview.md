# Libraries: `attach #name`

The core of Spaghetti is always available. Everything else is a library, and you add it to a program with `attach`.

- Core built-ins start with `@`, such as `@math`, `@file` and `@json`. They are always there.
- Libraries are installed with `attach #name`, and then they are used without the `@`. For example, `attach #ui` gives you `ui`.

```javascript
attach #ui

let win = ui.window("hello", 320, 180)
```

You can also load your own file with a path in quotes. That form is covered in [modules.md](../about/modules.md#attaching-a-whole-file).

```javascript
attach "./mathlib.spg"
```

## Available libraries

| Library | What it is for |
|---|---|
| `ui` | Windows, shapes, text, images and keyboard and mouse input. See [ui.md](ui.md). |
| `ps` | Running PowerShell commands and scripts. See the section below. |
| `http` | Making web requests. |
| `terminal` | Asking questions in the terminal, drawing boxes and rules. |
| `regex` | Regular expressions. |
| `crypto` | Hashes, HMAC, encodings, random values and password encryption. See the section below. |
| `csv` | Reading and writing CSV files. |
| `toml` | Reading TOML files. Read only, for now. |
| `json5` | Reading JSON5 files. Read only, for now. |
| `sqlite` | Working with SQLite databases. |
| `socket` | Network sockets. |
| `cmd` | Running other programs and checking they exist. See the section below. |
| `test` | Writing tests. It prints a report with the file and line of each failure, and exits with code `1` when a test fails. |

The names of the functions in each library are listed in the editor. Type the library name followed by a dot, and the autocomplete shows what it offers.

## `ps`: PowerShell

The `ps` library runs PowerShell from a program. It needs PowerShell on the computer. It uses `pwsh` when it is available, and falls back to `powershell.exe`.

```javascript
attach #ps

let result = ps.run("Get-Date -Format yyyy")

if (result.success) {
    print("year: " + result.output)
} else {
    print("failed with code " + @into.string(result.code))
}
```

- `ps.run(command)` runs a command and returns a `PsResult` with `success`, `code`, `output` and `error`.
- `ps.runFile(path)` runs a `.ps1` file. The path is relative to the `.spg` file that calls it.
- `ps.version()` returns the version of PowerShell that was found.
- `ps.isAvailable()` checks that PowerShell answers, not only that it is installed.

If PowerShell is missing or broken, the call is a `throw`. A command that runs but fails is not an error. It returns `success: false`, and you read the details in `code` and `error`.

`run` takes its own timeout, so a long command does not depend on the program's `#> timeout`.

### Safe mode

When a program runs code you did not write, turn on safe mode. You can do it in the file, with the directive `#> safe-party` at the top, or when you run it, with the `--safe-party` flag:

```javascript
#> safe-party
attach #ps

let result = ps.runFile("check.ps1")
```

```javascript
spg run --safe-party main.spg
```

In safe mode:

- `ps.run` and `cmd.run` are off. Calling them is an error, because a program can do anything. `cmd.exists` still works, since it starts nothing.
- `ps.runFile` only accepts `.ps1` files inside the folder of the `.spg` file. Absolute paths and `..` are refused.
- `ps.version` and `ps.isAvailable` still work.

The flag can only tighten the rules. A file cannot switch it off with its own header, so whoever runs the program decides.

## `cmd`: running programs

The `cmd` library starts another program and waits for it. The program is started directly, with the arguments you give, and no shell reads them. So an argument cannot run anything you did not mean to pass.

```javascript
attach #cmd

let result = cmd.run("git", ["status"])

if (result.success) {
    print(result.output)
} else {
    print("failed with code " + @into.string(result.code) + ": " + result.error)
}
```

- `cmd.run(program, args)` runs the program and returns a `CmdResult` with `success`, `code`, `output` and `error`. `success` is true only when the code is `0`. A code of `-1` means the program was stopped by a signal.
- `cmd.run(program, args, ms)` does the same, but gives up after `ms` milliseconds and throws.
- `cmd.runWith(program, args, options)` takes a map of options:
  - `"dir"`: the folder to run in.
  - `"input"`: text given to the program as its input.
  - `"timeout"`: the timeout in milliseconds, as text, like `"3000"`.
  - `"env.NAME"`: a variable to set for the program, like `"env.LANG": "C"`.

  An unknown key is an error, so a typo does not pass silently.
- `cmd.exists(name)` checks that a program is there to run. It looks in `PATH`, and on Windows it also tries the usual extensions. A name with a folder in it is checked as a path.

A program that cannot be started, such as one that does not exist, throws. A program that runs and fails returns `success: false`.

Under safe mode, every `cmd.run` and `cmd.runWith` is off. See the safe mode section above.

## `crypto`: hashes, encodings and encryption

The `crypto` library works on text. A text is treated as its UTF-8 bytes. Hashes come back as lowercase hex.

```javascript
attach #crypto

print(crypto.sha256("abc"))
print(crypto.uuid())

let sealed = crypto.encrypt("my secret", "a long password")
print(crypto.decrypt(sealed, "a long password"))
```

- Hashes: `sha1`, `sha256`, `sha512` and `md5`. `md5` and `sha1` are fine for checksums, never for passwords.
- Keyed hashes: `hmacSha256(key, text)` and `hmacSha512(key, text)`, for signing and checking messages. The key comes first.
- Encodings: `base64Encode`, `base64Decode`, `base64UrlEncode` (URL-safe), `base64UrlDecode`, `hexEncode` and `hexDecode`. Decoding gives text, and bytes that are not valid UTF-8 are an error.
- Random values: `randomHex(n)` gives `n` random bytes as hex (from 1 to 4096), and `uuid()` gives a random version 4 UUID.
- Encryption: `encrypt(text, password)` returns base64 text. `decrypt(sealed, password)` gives the text back. The key comes from the password with Argon2id, and the text is sealed with AES-256-GCM. Every call uses a new salt and nonce, so the same text encrypted twice gives different results.

`decrypt` throws when the password is wrong or when the text was changed. The two cannot be told apart, and that is on purpose.

Encryption protects the text only as far as the password is strong. Use a long password, and store it somewhere safe. A lost password cannot be recovered.

## Creating a library

This section is for people who work on Spaghetti itself. A library is written in Rust. You add one file to `crates/spg-libs/src`, and you add one line to the list of modules in the same crate. The registry takes care of the lexer, the type checker and the editor support, so the new functions appear in autocomplete without extra work.

See `crates/spg-libs/src/ui.rs` and `crates/spg-libs/src/ps.rs` for working examples.
