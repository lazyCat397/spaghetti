# Numbers: `int`, sized integers and `float`

Spaghetti has integer types of several sizes and two float types. Most programs only need `int` and `float`. The other types are there when the size or the exact bits matter.

| type | range | notes |
|------|-------|-------|
| `int` | -9223372036854775808 to 9223372036854775807 | same as `i64` |
| `i8` | -128 to 127 | |
| `i16` | -32768 to 32767 | |
| `i32` | -2147483648 to 2147483647 | |
| `u8` | 0 to 255 | |
| `u16` | 0 to 65535 | |
| `u32` | 0 to 4294967295 | |
| `u64` | 0 to 18446744073709551615 | unsigned, all 64 bits |
| `float` | 64-bit floating point | same as `f64` |
| `f32` | 32-bit floating point | |

`i64` is `int` and `f64` is `float`, with another name. `type(x)` prints `int` and `float` for them. For the sized integers and `f32`, `type(x)` prints their own name: `i8`, `u64`, `f32`.

## Literals

A whole number is an `int`: `42`. A number with a decimal point is a `float`, even when the decimal part is zero: `42.0`. Mixing them is an error, so `1` is not a float. Write `1.0`, or convert with `@into.float(1)`.

A literal can take the type it is used with. It is checked against that type's range, at compile time:

```javascript
let small: i8 = 100
const LIMIT: u8 = 255
print(small + 27)        // 127
print(small + 200)       // compile error: the value 200 does not fit i8
```

A whole-number literal next to a sized value takes that value's type, so `x + 120` works when `x` is an `i8`. A variable of type `int` is never converted on its own.

A literal above `9223372036854775807` is only allowed where a `u64` is expected:

```javascript
let big: u64 = 18446744073709551615
print(big)               // 18446744073709551615
let wrong: int = 9223372036854775808
// compile error: the value 9223372036854775808 does not fit int
```

## Mixing types

Different types never mix without a conversion. `i8 + int`, `u8 + i8` and `u64 + int` are type errors. Convert one side with `@into` (see below).

## Overflow

Arithmetic on any sized type wraps around its range by default. The value that leaves the range comes back at the other end:

```javascript
let a: i8 = 100
print(a + 100)           // -56
let b: u8 = 250
print(b + 10)            // 4
```

With `#> strict` at the top of the file, a result that leaves the range is an error instead:

```javascript
#> strict
let a: i8 = 100
print(a + 100)
// runtime error: addition overflow in i8: the result is outside i8's range -128..=127 (#> strict)
```

`int` wraps at 64 bits in the same way. Division by zero is always an error.

## `u64`

`u64` keeps all 64 bits. Its values are unsigned, so:

- comparisons are unsigned: `big > 9223372036854775807` is true for a large `u64`;
- division and remainder are unsigned;
- arithmetic wraps at 2^64: `u64::max + 1` is `0`;
- printing shows the number without a sign;
- `@array.sort` orders an `array<u64>` as unsigned numbers.

```javascript
const MAX: u64 = 18446744073709551615
print(MAX)               // 18446744073709551615
print(MAX / 2)           // 9223372036854775807
print(MAX + 1)           // 0
```

Converting a `u64` to `int` is an error when the value is above `9223372036854775807`, because it does not fit. Use `@into.int` only for values that fit.

## `f32`

`f32` is a 32-bit float. Every operation rounds its result to 32 bits, so the values are the ones a 32-bit float would give:

```javascript
let x: f32 = 0.1
let y: f32 = 0.2
print(x + y)             // 0.3
print(x == 0.1)          // true
```

Printing an `f32` shows the shortest text that reads back as the same `f32`, so `0.1` prints as `0.1`, not as the long 64-bit form. Arrays and maps of `f32` and `u64` print the same way.

A decimal literal next to an `f32` takes `f32`. Mixing `f32` with `float` is a type error.

## Converting with `@into`

Each sized type has a conversion. It takes an `int` (or another integer type) and gives the sized value:

```javascript
print(@into.u8(200))     // 200
print(@into.u8(300))     // compile error: the value 300 does not fit u8 ... use @into.u8w to wrap it
print(@into.u8w(300))    // 44
print(@into.i8w(200))    // -56
```

The `w` names wrap into the range. `@into.u64w(-1)` is `18446744073709551615`. The names without `w` are checked: a value that does not fit is an error, and a literal is checked at compile time.

The reverse conversion is `@into.int(x)`. It gives any sized integer back as an `int`. For a `u64` above `9223372036854775807` it is an error.

Floats convert from `u64` and from `f32` too:

```javascript
const MAX: u64 = 18446744073709551615
print(@into.float(MAX))                    // 1.8446744073709552e19
print(@into.f32(MAX))                      // 18446744000000000000.0
let x: f32 = 0.1
print(@into.float(x))                      // 0.10000000149011612
```

`@into.f32(x)` rounds a `float` to `f32` once. `@into.int(x)` on an `f32` drops the decimal part, as for `float`.

## Properties

Each sized integer has `max`, `min` and `bits`. Each one has the type of its own value:

```javascript
print(i8::max)          // 127
print(i8::min)          // -128
print(u16::max)         // 65535
print(u64::max)         // 18446744073709551615
print(i32::bits)        // 32
```

`f32` has `max`, `min` (the most negative value), `epsilon`, `inf` and `nan`. `int` and `float` have their own list: see [What's new](../whats-new.md).

## Math

The `@math` functions accept `f32`. The argument is read as a `float` (no loss), the function runs in 64 bits, and a float result comes back as `f32`, rounded once. Functions that return an `int` (such as `floor`) still return an `int`.

```javascript
let x: f32 = 2.0
print(@math.sqrt(x))     // 1.4142135
```

## Limits

- `u64` is not converted to `float` by `@math` or by arithmetic. Use `@into.float` first.
- Literals passed to a builtin function are not converted to the parameter's type: `@array.contains(xs, 1)` with `xs` an `array<u64>` is a type error. Give the value a sized type first, with a constant: `const ONE: u64 = 1`.
- `@array.sum` accepts only `array<int>`.
- `pow` on `f32` is computed in 64 bits and then rounded, so in rare cases it can differ from a 32-bit power.
- Floats are printed with `.0` when they are whole: `3.0`, not `3`.
