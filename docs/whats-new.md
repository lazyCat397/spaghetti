# What's New in Spaghetti

The latest changes to the language, newest first. Each section has a short
example. For a full walkthrough of every feature, see `examples/`.

## Smaller builds and compact formatting

`spg build --lean` makes an executable of about 3 MB instead of 13 MB. It carries a runtime without the JIT and without the heavy libraries, so it runs small programs that need only the core libraries, a few times slower in loops. Programs with a window get a runtime of about 4 MB. See [Building a standalone program](installation/tools.md#building-a-standalone-program).

`spg build --enc` renames the names of a program's functions and variables, and removes its comments, so the built executable is hard to read. See [Renaming names](installation/tools.md#renaming-names-enc).

Standard library modules (`attach #cmp`) now work in executables built with `spg build`, when the executable runs somewhere the standard library folder doesn't exist.

`spg fmt --min` writes a compact layout, and `spg fmt --min-nc` writes the same without comments. The program is checked before the file is written, so the compact text is the same program.

```javascript
let arr = [1, 2, 3]
print(arr)
```

becomes `let arr=[1,2,3];print(arr)`.

## Generic functions

A function can take type parameters, and its body is checked for each type it is called with. A bound says which types are allowed: a union such as `int or float`, or `Numeric`, which is every integer and float type.

```javascript
fn add<T: Numeric>(n: T) -> T {
    return n + T.from(1)
}

print(add(3.4), add(5), add(-3.2))   // 4.4 6 -2.2
```

Inside a generic, `T.from(1)` converts a literal to `T` exactly. Nothing is converted behind your back: `1 + 1.0` is still an error. The full rules, and what is not there yet (methods and explicit type arguments), are in [Functions and lambdas](about/functions-and-lambdas.md#generic-functions).

## Unknown type names are errors

A type name that is not a builtin, a struct or an enum of the file is an error where it is written, in a field, a `let`, a parameter or a return type. Before, the mistake only showed up as a confusing mismatch at the value.

```javascript
struct Caixa {
    v: Foo        // error: unknown type 'Foo'
}
```

## Sized integers and `f32`

There are now integer types of several sizes: `i8`, `i16`, `i32`, `u8`, `u16`, `u32` and `u64`. There is also `f32`. `i64` is another name for `int`, and `f64` for `float`. Each sized type checks its range, and the full table, the overflow rules and the conversions are in [Numbers](standard-lib/numbers.md).

```javascript
let small: i8 = 100
print(small + 100)               // -56, the result wraps around the range
let big: u64 = 18446744073709551615
print(big)                       // 18446744073709551615
print(@into.u8w(300))            // 44
let x: f32 = 0.1
print(x)                         // 0.1
print(type(small))               // i8
```

`type(x)` gives the sized name (`i8`, `u64`, `f32`) for these types. `@array.sort` orders an `array<u64>` as unsigned numbers.

## One form for name lists and `for`

Names in `use` and `public ... from` always go in parentheses, and so does every `for` header. The forms without parentheses are errors now.

```javascript
use (a, b) from "x.spg"
from "x.spg" use (a, b)
public (a, b) from "x.spg"

for (x in [1, 2, 3]) { print(x) }
for (i, v in ["x", "y"]) { print("${i}: ${v}") }
for (k in 1..4 step 2) { print(k) }
```

`use a, b from "x.spg"` and `for x in list {` no longer compile. The error says which parenthesis is missing.

## `namespace`

A namespace groups functions and static values under one name. It has no instances, so it is the right tool for helpers that belong together:

```javascript
public namespace som {
    let calls;
    fn add(a: int, b: int) -> int {
        calls += 1
        return a + b
    }
}

print(som.add(1, 2))   // 3
print(som.calls)       // 1
som.calls = 0          // error: namespace 'som' is read-only from outside
```

Statics are readable from outside, writable only from inside. Inside a file, every member is visible; from another file, only the members marked `public` are. Details are in the Namespaces section of the structs and enums page. This replaces the old `struct namespace {}` pattern: `namespace` is now a keyword, so a struct or variable with that name must be renamed.

## `float::inf`, `float::nan` and `isNaN`

```javascript
let best = float::inf                // start above every float
for (v in [3.0, 1.5, 2.0]) {
    if (v < best) {
        best = v
    }
}
print(best)                          // 1.5

print(float::nan == float::nan)      // false: NaN is not equal to anything
print(float::nan.isNaN())            // true
```

`float::inf` and `float::nan` join the type properties below. `@math.isNaN` works
as a function and as a method on floats, like the rest of `@math`. It runs in the
JIT too.

## Type properties with `::`

Constants that belong to a type are read with `::`. They are resolved when the
program is compiled, so they cost nothing at run time.

```javascript
print(int::max)          // 9223372036854775807
print(int::min)          // -9223372036854775808
print(float::max)        // 1.7976931348623157e308
print(float::min)        // -1.7976931348623157e308
print(float::epsilon)    // 2.220446049250313e-16

let limit: int = int::max - 1
```

`int::max`, `int::min`, `int::bits`, `float::max`, `float::min` and `float::epsilon` are the
whole list for `int` and `float`. The sized integers and `f32` have their own, such as `u8::max` and `f32::epsilon`: see [Numbers](standard-lib/numbers.md). Any other name is a compile error. Properties are read only:
`int::max = 5` does not compile. The `.` operator does not change: `::` is a
separate operator.

Very large and very small floats now print in scientific notation: `print`
shows `1.7976931348623157e308` instead of 309 digits, and `print(1e20)` shows
`1e20`. Only the printed text changed, not the number. A float turned into a
string keeps the old form, also inside an array or a map: `"${x}"`,
`@into.string(x)` and `@into.string([x])` all give `100000000000000000000.0`.
Completion after `int::` and `float::` lists the properties, with their values.

## Methods on value types

Methods on `int`, `float`, `bool` and `string` are now declared with `int::methods.add(fn name(...) { ... })` and the like. `me` is the value the method is called on and is always implicit. The old `fn string.shout(s: string)` form is gone: it is now a compile error, with the new form in the message.

```javascript
string::methods.add(fn shout() -> string {
    return "${me}!"
})
int::methods.add(fn double() -> int {
    return me * 2
})

print("hello".shout())        // hello!
print(21.double())            // 42
print(string::methods)        // the built-in names, then the added ones
```

`string::methods` is a snapshot array of method names. The call is only at the top level of a file, and `public` in front makes a method visible to the files that attach its module. See [functions](about/functions-and-lambdas.md#methods-on-value-types).

## Destructuring `let`

```javascript
let (first, second) = [10, 20]
print(first + second)   // 30

struct Point {
    x: int,
    y: int
}

let point = Point { x: 3, y: 4 }
let { x, y } = point
print(x * y)            // 12
```

Parentheses take array items by position. Braces take struct fields by name.
`public let (...)` shares each name. Only `let` works this way: `const` takes one
name. An array shorter than the names stops the program with an index error.

## Number literals

```javascript
let population = 1_000_000   // underscores between digits
let mask = 0xFF_00           // hex: 65280
let flags = 0b1010_0001      // binary: 161
let big = 1e6                // exponent: a float, 1000000.0
let small = 2.5e-3           // 0.0025
```

- Underscores are only for reading: they must sit between two digits.
- `1e6` is a `float`. For an int, write `1_000_000`.
- A number that cannot be read, like `0b102` or a value too big for its type,
  is a lex error with its line and column.

## Unions (`A or B`)

A type can be one of several types. Works anywhere a type is written: parameters,
return values, `let`, struct fields, and `array<...>`.

```javascript
fn show(value: string or float) {
    print("${value}")
}

show("text")
show(3.5)
```

- Duplicates collapse: `int or int` is just `int`.
- Nested unions flatten, and order does not matter for equality.
- Use `(A or B)?` for an optional union. `A or B?` means `A or (B?)`.
- `none` is not a member of a union. Use `?` instead.
- There is no narrowing: arithmetic on a union is an error.

## `type(x)`

Returns the type of a value as a string: `int`, `float`, `bool`, `string`,
`array`, `map`, `fn`, `task`, `none`, or the declared name of a struct or enum.
For the sized integers and `f32`, it gives their own name: `i8`, `u64`, `f32`.

```javascript
if (type(value) == "string") {
    print("got text")
}
```

## Typed literals

Array and map literals take the type expected by their context: an annotated
`let`, an assignment, a `return`, a field, or a named function argument.

```javascript
let scores: array<int> = [10, 20, 30]
```

Errors are specific: `this array's items are int, found string`.

## Increment and decrement (`++` / `--`)

Usable as expressions, prefix or postfix, on variables, indexes, and fields.

```javascript
let i = 0
let values = [5, 6, 7]
let next = values[i++]
```

Note: `--x` and `5--3` now mean decrement. For double negation, add a space:
`- -x`.

## `%=`

Compound modulo assignment is back: `x %= 3`.

## Zero values with `let x;`

`let x;` declares a variable with the zero value of its type: `0` for an
unannotated int, `0.0`, `false`, `""`, `none`, `[]`, or `{}`. Structs, enums,
unions, and functions have no zero value. `const` always needs a value.

## Use and public with parentheses

```javascript
use (farewell) from "lib/farewell.spg"
public (greet) from "lib/greet.spg"
```

The keywords changed: `export` is now `public`, `import` is now `use`, and `install` is now `attach`. `attach "file.spg"` brings every public name of a file, and `attach #ui` still attaches a library. Blocks still use braces.

## Loops

```javascript
let names = ["ana", "bia"]
for (i, name in names) {
    print("${i}: ${name}")
}

let ages = ["ana": 30, "bia": 25]
for (name, age in ages) {
    print("${name} is ${age}")
}
```

- `for (i, v in array)` gives the position and the element.
- `for (k, v in map)` gives the key and the value, in insertion order. Both names are required.
- `do { } while (cond)` is a loop that runs at least once. `do` is now a reserved word.
- `continue` jumps to the condition check.

## Membership: `in`

```javascript
if (x in list) { }
if (key in map) { }
```

The checker turns these into `@array.contains` and `@map.has`.

## Functional helpers

- `@array.map`, `@array.filter`, `@array.reduce`: take a typed lambda, `fn(x: T) -> U`.
- `@array.first` and `@array.last`: return `T?`, so an empty array gives `none`.
- `@map.map` and `@map.filter`: take `fn(k: K, v: V) -> U` and `fn(k: K, v: V) -> bool`, in insertion order.

Full example: `examples/13_loops_and_operators.spg`.

## Other

- `@str.slice(text, start, end)` is a second name for `@str.substring`. The start is included and the end is not.
- `spg --version` now shows the build id, so you can tell which binary is running:
  `spg 1.0.0 (a1b2c3d)`. Use it when two copies of `spg` are installed.
