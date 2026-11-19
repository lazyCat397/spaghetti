# System and time: `@sys` and `@time`

This page covers two namespaces:

- `@sys` talks to the program's surroundings: command-line arguments, environment variables, the standard input, the folder it runs in, and its exit code.
- `@time` gives the current time, measures how long the program has been running, and pauses it.

## `@sys`

### Command-line arguments

**`@sys.args()`** returns the arguments given to the program, as an `array<string>`. Pass them after `--` when you run `spg`:

```javascript
// greet.spg
print(@sys.args())

// spg greet.spg -- one "two words" --flag
// [one, two words, --flag]
```

Everything after the first `--` goes to the program, including things that look like options. A second `--` is passed through as an ordinary argument:

```javascript
// spg prog.spg -- a -- b
// [a, --, b]
```

Without `--`, an extra word after the file name is an error for `spg`, not a program argument.

### Environment variables

**`@sys.env(name, fallback)`** returns the value of an environment variable. If the variable does not exist, or its value is not valid text, it returns `fallback`:

```javascript
print(@sys.env("HOME_NOT_SET_HERE", "fallback"))   // fallback
print(@sys.env("PATH", "none") != "none")          // true
```

### Standard input

**`@sys.readLine()`** reads one line from the standard input, without the line break. When there is no input left, it is a runtime error:

```javascript
let name = @sys.readLine()
print("hello " + name)
```

```
// printf 'Ana\n' | spg greet.spg
// hello Ana
```

**`@sys.readAll()`** reads everything that is left, and returns it as one string. At the end of the input it returns `""` and is not an error:

```javascript
let first = @sys.readLine()
let rest = @sys.readAll()

print(first, rest == "")
```

To ask a question and wait for an answer, use the global `readLine(prompt)` instead. It writes the prompt without a line break first.

Both functions write out anything printed so far before they wait, so a prompt shows up before the program waits for you.

When there is no input, `readLine` is an error, and you can catch it:

```javascript
try {
    print(@sys.readLine())
} catch (e) {
    print("caught: " + e)
}
// caught: @sys.readLine: end of input (nothing left on stdin)
```

### Platform and folder

**`@sys.platform()`** returns the name of the operating system, such as `"linux"`, `"windows"` or `"macos"`:

```javascript
print(@sys.platform())   // linux
```

**`@sys.cwd()`** returns the folder the program runs in:

```javascript
print(@sys.cwd() != "")   // true
```

### Ending the program

**`@sys.exit(code)`** ends the program right away with that exit code. Anything printed before it is shown:

```javascript
print("before")
@sys.exit(3)
print("after")   // never runs

// before
// exit code: 3
```

`exit` cannot be caught. Even inside `try`, the program ends:

```javascript
try {
    @sys.exit(4)
} catch (e) {
    print("never")
}

// exit code: 4
```

Exit codes are passed to the operating system, which keeps only the last 8 bits. So `300` becomes `44`, and `-1` becomes `255`. Use codes from `0` to `255`.

## `@time`

### Current time

**`@time.now()`** returns the current date and time in UTC, as text in ISO-8601 format:

```javascript
print(@time.now())   // 2026-10-11T21:36:44Z
```

**`@time.unix()`** returns the number of milliseconds since 1970-01-01 (the Unix epoch), as an `int`. This is the same unit as `@file.modified`.

```javascript
print(@time.unix() > 1700000000000)   // true
```

### Measuring time

**`@time.elapsed()`** returns the time since the program started, as text. It uses units from the largest to the smallest, and it shows at most two of them:

```javascript
print(@time.elapsed())   // 50ms 608µs
```

The units are `h`, `m`, `s`, `ms`, `µs` and `ns`. It shows the largest unit that fits, plus the next one only if it is not zero:

- `45ms` for 45 milliseconds
- `3s 250ms` for 3 seconds and 250 milliseconds
- `1h 2m` for 1 hour, 2 minutes and 3 seconds (the seconds are left out)
- `1h` for 1 hour and 5 seconds, because the minutes are zero

`elapsed` returns text, so you cannot subtract two of its results. To measure a piece of code, take the difference of two `@time.unix()` values, which are plain integers:

```javascript
let start = @time.unix()

// work here

let took = @time.unix() - start
print("took " + @into.string(took) + " ms")
```

### Pausing

**`@time.sleep(ms)`** pauses the program for `ms` milliseconds. It takes an `int`, and `0` is allowed. A negative value is a runtime error:

```javascript
print(@time.elapsed())
@time.sleep(1100)
print(@time.elapsed())   // about 1s 100ms more than before
```

The program stops completely during the pause. Nothing else runs.

### Errors

Negative sleep:

```javascript
@time.sleep(-1)
// runtime error: @time.sleep expects a non-negative number of milliseconds, got -1
```

Mixing types:

```javascript
print(@time.unix() + "x")
// operator Add not supported for int and string
```

## Limits

- `@sys` does not run other programs, and it has no process id.
- `@time` has no local time zone. `now()` and `modifiedDate` use UTC.
- `elapsed` has no picosecond unit, and it shows at most two units.
