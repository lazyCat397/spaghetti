# Errors and debugging

This page covers the kinds of errors a Spaghetti program can have, how to raise your own errors with `throw`, how to catch them with `try` and `catch`, how to read a stack trace, and the file directives that change how errors are shown.

## Kinds of errors

Spaghetti finds problems at three different stages:

- **Source errors** are found before the program runs: a missing quote, a wrong type, a name that does not exist. The program does not start.
- **Runtime errors** happen while the program runs: a division by zero, an index that does not exist, a file that is missing.
- **Thrown errors** are the ones your own code raises with `throw`.

A runtime error or a thrown error stops the program, unless a `try` catches it. The next sections explain how.

Source errors look like this:

```javascript
let x: int = "a"
// type error in main.spg at line 1, column 1: declared type is int but value is string
```

Runtime errors look like this:

```javascript
print(10 / 0)
// runtime error: division by zero
//   at <main> (main.spg:1)
```

## Throwing errors

`throw` raises an error with a message. The message must be a `string`:

```javascript
fn check(n: int) -> int {
    if (n < 0) {
        throw "negative: " + @into.string(n)
    }

    return n * 2
}
```

Throwing anything other than a string is a type error:

```javascript
throw 5
// 'throw' takes a string (the message), found int; build one with @into.string(...) or '+'
```

A `throw` that nobody catches stops the program:

```javascript
throw "boom"
// runtime error: unhandled exception: boom
//   at <main> (main.spg:1)
```

## Catching errors

Put the code that might fail inside `try`. Put the reaction inside `catch`. The name in parentheses holds the message as a `string`:

```javascript
try {
    print(check(4))
    print(check(-1))
    print("not reached")
} catch (e) {
    print("caught: " + e)
}

// 8
// caught: negative: -1
```

When the error happens, the rest of the `try` block is skipped. The program continues after the `catch` block.

### Errors cross function calls

A `throw` inside a function is caught by the `try` of the code that called it, no matter how many calls are in between. The error can also happen in the middle of an expression, and the expression is skipped:

```javascript
let x = 0

try {
    x = 100 + check(-5) * 2
} catch (e) {
    x = -1
}

print(x)   // -1
```

`x` is never set to the result of the expression, because `check` throws before the expression finishes.

### Rethrowing

Inside `catch`, you can throw again. The new error goes to the next outer `try`:

```javascript
try {
    try {
        throw "inner"
    } catch (e) {
        throw "rethrown " + e
    }
} catch (e) {
    print(e)   // rethrown inner
}
```

### Loops and returns

`break` and `continue` work inside `try`. Leaving a `try` this way is safe, and any later `throw` is caught by the right handler:

```javascript
let i = 0

while (i < 5) {
    i += 1

    try {
        if (i == 2) {
            continue
        }
        if (i == 4) {
            break
        }
    } catch (e) {
        print("never")
    }
}

print(i)   // 4
```

`return` inside `try` also works as expected:

```javascript
fn f() -> int {
    try {
        return 1
    } catch (e) {
        return 2
    }
}

print(f())   // 1
```

### Catching runtime errors

Runtime errors can be caught the same way as thrown ones. The message says what happened:

```javascript
try {
    print(10 / 0)
} catch (e) {
    print(e)   // division by zero
}

let a: array<int> = [1]

try {
    print(a[9])
} catch (e) {
    print(e)   // index out of bounds: 9 (array has 1 element(s))
}

let m: map<string, int> = [:]

try {
    print(m["k"])
} catch (e) {
    print(e)   // key not found in map: "k" (use @map.has(m, k) or @map.get(m, k, fallback))
}
```

A few errors cannot be caught. `@sys.exit` ends the program even inside `try`:

```javascript
try {
    @sys.exit(4)
} catch (e) {
    print("never")
}

// the program exits with code 4
```

### No `finally`

Spaghetti has no `finally` block. If you need code to run after a `try`, put it after the `catch`, or inside both branches.

### Limits of `throw`

- Only `string` can be thrown. There is no custom error type, and `catch` cannot filter by type.
- `catch (e)` always receives a `string`.

## Stack traces

A runtime error that is not caught prints a stack trace. It lists the functions that were running when the error happened, from the innermost to the outermost:

```javascript
fn divide(a: int, b: int) -> int {
    return a / b
}

fn compute(x: int) -> int {
    return divide(10, x)
}

print(compute(0))
```

```
runtime error: division by zero
  at divide (main.spg:2)
  at compute (main.spg:6)
  at <main> (main.spg:10)
```

Each line shows a function and the line where it was at that moment. `<main>` is the top level of the file.

When one line repeats many times in a row, the trace shows it once, with a count:

```
  at down (main.spg:2)
  ... same line repeated 49 more times
```

The trace stops after 50 frames, and it shows how many more were left out.

A `throw` that is not caught also prints a trace, and so does a runtime error that happens inside a module. The file name is the module's file, not the file that did the `use`.

Stack traces are not shown for errors inside a `try` that catches them, because the program keeps running.

Traces show the line of the statement, not the exact column.

### Infinite recursion

A function that calls itself forever stops with a stack overflow error, after 100,000 nested calls:

```javascript
fn down(n: int) -> int {
    return down(n + 1)
}

print(down(0))
```

```
runtime error: stack overflow: more than 100000 nested calls (is a function calling itself forever?)
  at down (main.spg:2)
  ... same line repeated 49 more times
  ... 99950 more frames
```

Like the other runtime errors, a stack overflow can be caught with `try`.

## File directives

A directive is a line that starts with `#>` at the top of a file. It changes how the file is run or checked. Directives apply to the file you run, not to the files it imports.

Directives must come before any code. Blank lines and comments can come before them:

```javascript
// my program
#> friendly-terminal
print("hello")
```

The `#>` and the name can be written with or without a space: `#>strict` and `#> strict` mean the same thing.

Each directive can appear only once.

### `friendly-terminal`

This directive changes the tone of the error messages. They start with `Oops!` and explain what to check:

```javascript
#> friendly-terminal
let x = 10 / 0
```

```
error: Oops! division by zero. The program stopped while running. Check the values involved at that point; a try/catch can handle this kind of error.
  at <main> (main.spg:2)
```

The message has the same facts as the normal one. Only the tone and the hint change. Use it when you are learning, or when a plain message is hard to read.

### `timeout`

`timeout` stops the program if it runs for longer than the number of milliseconds you give:

```javascript
#> timeout 200
while (true) {
}
```

```
runtime error: timeout: the program ran for more than 200 ms
```

The time counts from the moment the program starts running. It also stops a program that is waiting in `@time.sleep` or `readLine`. The value must be a whole number greater than `0`.

A timed-out program exits with code `124`.

### `strict`

`strict` makes the checker stricter. The program still compiles the same way, but three things become errors:

1. **Integer overflow.** Normally an `int` wraps around when it goes past its limit. With `strict`, the operation is a runtime error:

```javascript
#> strict
let big: int = 9223372036854775807
print(big + 1)
// runtime error: integer overflow in addition: the result does not fit in an int (64 bits, #> strict)
```

This error can be caught with `try`.

2. **Shadowing.** A name that hides a name from an outer scope is an error. This includes a parameter or a local with the same name as a global:

```javascript
#> strict
let n = 1
fn f(n: int) -> int { return n }
// 'n' hides a name from an outer scope (not allowed with #> strict)
```

Names that start with `_` are allowed to shadow, so `fn f(_n: int)` is fine.

3. **Unreachable code.** Code after `return`, `break`, `continue` or `throw` in the same block is an error:

```javascript
#> strict
fn f() -> int {
    return 1
    print(2)
}
// code after 'return' can never run (not allowed with #> strict)
```

### `safe-party`

This directive turns on the limits for the `ps` and `cmd` libraries. `ps.run` and `cmd.run` are off, and `ps.runFile` only accepts a `.ps1` file inside the folder of the script. Use it in a file that runs code you did not write. See [overview.md](../libraries/overview.md#safe-mode) for the details.

```javascript
#> safe-party
attach #ps

ps.run("Get-Date")   // throws: ps.run is off in safe-party mode
```

Whoever runs a file can also turn the limits on with `spg run --safe-party`, without changing the file.

### Directive errors

An unknown directive is an error. If the name is close to a real one, the message suggests it:

```javascript
#> friendly-terminl
// unknown directive '#> friendly-terminl' at line 1, column 1, did you mean '#> friendly-terminal'?
```

Because the directive is not recognized, the message uses the plain tone.

A directive after code is an error:

```javascript
print(1)
#> strict
// '#>' directive after code at line 2, column 1; directives must come before any code
```

A repeated directive is an error:

```javascript
#> strict
#> strict
// directive '#> strict' is written twice (duplicate at line 2, column 1)
```

`timeout` needs a number greater than `0`:

```javascript
#> timeout 0
// directive '#> timeout' expects a whole number of milliseconds greater than 0, like `#> timeout 5000` (line 1, column 1)
```

## Exit codes

When you run `spg`, the exit code says what happened:

| Code | Meaning |
|---|---|
| `0` | The program ran and finished. |
| `1` | An error in the program or the source file: a source error, an uncaught error, or a file that could not be read. |
| `2` | Wrong use of `spg`, such as an unknown option or an extra word after the file name. |
| `124` | The program was stopped by `#> timeout`. |

`@sys.exit(n)` sets the exit code to `n` yourself. See [system-and-time.md](system-and-time.md) for details.

## Limits

- Only `string` can be thrown, and `catch` always receives a `string`.
- There is no `finally`.
- Traces show lines, not columns.
- Directives apply only to the file you run.
