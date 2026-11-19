# Modules

A Spaghetti program can be split across several files. One file can use the functions, structs, enums and values of another file. This page covers how to share code between files: `public`, `use`, `attach` and re-exports.

## Sharing with public

A file keeps everything private by default. Put `public` in front of a declaration to share it:

```javascript
// mathlib.spg
public fn square(x: int) -> int {
    return x * x
}

public let version = "1.0"

public struct Vec {
    x: int,
    y: int
}

public enum Dir { North, South }

fn helper() -> int {
    return 42
}
```

`public` works on functions, `let`, `const`, structs and enums. `square`, `version`, `Vec` and `Dir` can be used from other files. `helper` cannot. It stays private to `mathlib.spg`.

## Using names with use

Use `use` with the names you want. The names always go in parentheses. The path is relative to the file that does the `use`, and it always ends in `.spg`:

```javascript
// main.spg
use (square, Vec) from "./mathlib.spg"

print(square(5))     // 25

let v = Vec { x: 1, y: 2 }
print(v.x + v.y)     // 3
```

You can use names from a file in a subfolder:

```javascript
use (shout) from "./utils/text.spg"
```

The path can also come first. `from "./mathlib.spg" use (square, Vec)` means exactly the same as the line above:

```javascript
from "./mathlib.spg" use (square, Vec)
```

Only named lists exist. You list each name you want. There is no `use *` and no default form.

Using a private name is an error:

```javascript
use (useHelper) from "./mathlib.spg"
print(helper())
// undefined function 'helper'
```

`helper` is private, so `main.spg` cannot see it. Only `public` names can be used from another file.

Asking for a name the file does not declare is also an error:

```javascript
use (nope) from "./mathlib.spg"
// module "./mathlib.spg" has no declaration named 'nope'
```

The path must end in `.spg`:

```javascript
use (square) from "./mathlib"
// module path "./mathlib" must end in '.spg'
```

## Re-exporting

Using a name does not share it. If `main.spg` uses `square` from `mathlib.spg`, another file that uses names from `main.spg` will not find `square` there. To pass a name on, write a `public ... from` line:

```javascript
// facade.spg
public (square, version) from "./mathlib.spg"
public (shout) from "./utils/text.spg"
```

Now another file can take the names from the facade, and it does not need to know where each one comes from:

```javascript
// app.spg
use (square, version, shout) from "./facade.spg"

print(square(3), version, shout("hi"))   // 9 1.0 HI!
```

If you take a name from a file that only uses it, the compiler points you to the fix:

```javascript
// fac2.spg
use (square) from "./mathlib.spg"
```

```javascript
// another file
use (square) from "./fac2.spg"
// 'square' is only used by module "./fac2.spg", not exported from it
// using a name doesn't re-export it; add `public (square) from "./mathlib.spg"` there
```

## Attaching a whole file

`use` lists names one by one. For a file with many public names, `attach` brings in all of them at once:

```javascript
attach "./mathlib.spg"

print(square(4), version)   // 16 1.0
print(Dir.South)            // South

let v = Vec { x: 3, y: 4 }
```

`attach` brings every `public` name of the file, including names that come from `public ... from` lines in it. Private names are not brought in.

You can use `use` and `attach` in the same file. Both are at the top level.

### Which name wins

When names clash, the rules are:

1. A name declared in the file itself wins over anything it attaches or uses:

```javascript
attach "./mathlib.spg"

fn square(x: int) -> int { return 100 }

print(square(2))   // 100, the local one
```

2. An explicit `use (name)` wins over `attach`:

```javascript
attach "./mathlib.spg"
attach "./other.spg"

use (square) from "./mathlib.spg"

print(square(2))   // from mathlib, picked by the use
```

3. Attaching the same file twice causes no conflict:

```javascript
attach "./mathlib.spg"
attach "./mathlib.spg"

print(square(6))   // 36
```

4. Two attaches that bring the same name from different files are only an error if the name is used. Here `square` comes from two files and is used, so the compiler stops and tells you to resolve it with a `use`:

```javascript
attach "./mathlib.spg"
attach "./amb1.spg"

print(square(2))
// 'square' is attached from both "./mathlib.spg" and "./amb1.spg" and used in this file;
// use it by name to pick one
```

If `square` were never used, the program would still run.

### Where names are checked

A name can come from a `use` or from a `attach`, but not from a local declaration as well:

```javascript
use (square) from "./mathlib.spg"
fn square(x: int) -> int { return 100 }
// 'square' conflicts with a local declaration
```

Using the same name twice from `use` is also an error:

```javascript
use (square) from "./mathlib.spg"
use (square) from "./facade.spg"
// 'square' is imported more than once
```

`attach` needs the file path with `.spg` too:

```javascript
attach "./mathlib"
// module path "./mathlib" must end in '.spg'
```

## Cycles are errors

If two files use each other, directly or through a chain, the compiler stops and shows the whole loop:

```javascript
// ca.spg
use (pb) from "./cb.spg"

public fn pa() -> int { return pb() }
```

```javascript
// cb.spg
use (pa) from "./ca.spg"

public fn pb() -> int { return 1 }
```

```
error: circular use: ca.spg -> cb.spg -> ca.spg
```

Move the shared part into a third file that neither of them depends on.

## Types across files

Types work the same as in one file. A struct or enum from another file can be used as a parameter, a return value, a field, or inside an array:

```javascript
use (Vec, Dir) from "./mathlib.spg"

fn length2(v: Vec) -> int {
    return v.x * v.x + v.y * v.y
}

let d: Dir = Dir.North
```

## Limits

- Only named lists (`use (a, b)`) and `attach` exist. There is no namespace import.
- Re-exports are explicit: using a name does not pass it on.
- Cycles are not allowed.
