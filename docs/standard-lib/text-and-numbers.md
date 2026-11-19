# Text and numbers: `@str`, `@math` and `@into`

This page covers the built-in functions for working with strings and numbers, and for converting between types.

- `@str` works with text.
- `@math` works with numbers.
- `@into` converts a value from one type to another.

Each function can be called in the usual way, `@str.trim(s)`, or as a method on the value, `s.trim()`. The method form puts the value first. Both forms do the same thing.

## Strings

Strings are written with double quotes, and they can be joined with `+` only when both sides are strings:

```javascript
let greeting = "Hi " + "there"
print(greeting)   // Hi there

print("x" + 1)
// operator Add not supported for string and int
```

To join a number, convert it first with `@into.string` (see below).

### Escapes

Inside double quotes, these escapes work:

```javascript
print("say \"hi\"")   // say "hi"
print("back\\slash")  // back\slash
print("line\tone")    // a tab between "line" and "one"
print("a\nb")         // two lines: a, then b
```

### Interpolation

Put an expression inside `${ }` to insert its value into the string:

```javascript
let name = "Ana"
let age = 30

print("Hi ${name}, you are ${age + 1}")   // Hi Ana, you are 31
```

A `$` that is not followed by `{` is just a character:

```javascript
print("cost: $5")   // cost: $5
```

### Raw strings

Single quotes make a raw string. Nothing inside is interpreted, so there are no escapes and no interpolation:

```javascript
print('raw ${name} stays as is')   // raw ${name} stays as is
print('print("hi")')               // print("hi")
print('a\nb')                      // a\nb (backslash and n, not a new line)
```

Raw strings are useful for text that contains a lot of quotes or backslashes, like code or paths.

### Length

`@str.len` counts characters, not bytes. So accented letters count as one each:

```javascript
print(@str.len("olá"))   // 3
print("".len())          // 0
```

### Changing case and spaces

```javascript
print(@str.toUpper("abc"))       // ABC
print(@str.toLower("ABC"))       // abc
print(@str.trim("  Hello  "))    // Hello
print(@str.trimStart("  Hello")) // Hello
print(@str.trimEnd("Hello  "))   // Hello
```

`trim` removes spaces from both ends. `trimStart` and `trimEnd` remove them from one end only.

### Padding

`padStart` and `padEnd` make a string at least a given length. The default fill is a space. You can pass a different fill:

```javascript
print(@str.padStart("7", 3))          //   7
print(@str.padStart("7", 3, "0"))     // 007
print(@str.padEnd("ab", 4, "."))      // ab..
```

If the string is already long enough, it is returned as is.

### Searching

`contains`, `startsWith` and `endsWith` answer yes or no:

```javascript
print(@str.contains("Hello", "ell"))    // true
print(@str.startsWith("Hello", "He"))   // true
print(@str.endsWith("Hello", "lo"))     // true
```

`indexOf` gives the position of the first match, and `lastIndexOf` gives the last one. Positions count characters, and they start at `0`. If nothing matches, both return `-1`:

```javascript
print(@str.indexOf("hello", "l"))       // 2
print(@str.lastIndexOf("hello", "l"))   // 3
print(@str.indexOf("hello", "z"))       // -1
```

### Taking parts of a string

`charAt` gives one character as a string:

```javascript
print(@str.charAt("Hello", 1))   // e
```

`substring` takes from a start position up to an end position. The end is not included:

```javascript
print(@str.substring("Hello", 1, 4))   // ell
```

`slice` is another name for `substring`, with the same rules. It is handy when you come from languages that call it that:

```javascript
print(@str.slice("Hello", 1, 4))   // ell
```

Asking past the end of the string is a runtime error. Check the length first if you are not sure.

`split` breaks a string into an array. Empty pieces are kept:

```javascript
print(@str.split("a,b,c", ","))   // [a, b, c]
print(@str.split("a,,b", ","))    // [a, , b]
print(@str.split("abc", ""))      // [a, b, c]
```

### Replacing and repeating

`replace` changes every match:

```javascript
print(@str.replace("a-b-c", "-", "+"))   // a+b+c
```

`repeat` joins copies of a string:

```javascript
print(@str.repeat("ab", 3))    // ababab
print("A".repeat(0) + "|")     // |
```

### Type errors

Passing a number where a string is expected:

```javascript
print(@str.toUpper(42))
// '@str.toUpper' argument #1 expects string, got int
```

## Numbers

The sized integers (`i8` to `u64`) and `f32` are described in [Numbers](numbers.md). `i64` and `f64` are another name for `int` and `float`.

Spaghetti has two number types, `int` and `float`. They never mix on their own. You cannot add an `int` to a `float`, and you cannot pass an `int` where a `float` is expected:

```javascript
print(@math.sqrt(16))
// '@math.sqrt' argument #1 expects float, got int
```

Write `16.0` instead, or convert the value with `@into.float` (see below).

A float always shows its fraction, even when it is zero, so it never looks like an int:

```javascript
print(2.0)              // 2.0
print(0.1 + 0.2)        // 0.30000000000000004
print(1.0 / 3.0)        // 0.3333333333333333
```

### Number literals

Underscores make long numbers easier to read. They must sit between two digits:

```javascript
let population = 1_000_000
print(population)       // 1000000
```

Hex (`0x`) and binary (`0b`) literals are ints:

```javascript
print(0xFF_00)          // 65280
print(0b1010_0001)      // 161
```

Scientific notation is a float, so `1e6` prints as `1000000.0`. For an int, write
`1_000_000`:

```javascript
print(1e6)              // 1000000.0
print(2.5e-3)           // 0.0025
```

### Basic math

```javascript
print(@math.abs(-5))          // 5
print(@math.abs(-2.5))        // 2.5
print(@math.pow(2.0, 10.0))   // 1024.0
```

`abs` works with either type and gives the same type back.

Rounding turns a `float` into an `int`:

```javascript
print(@math.floor(2.7))    // 2
print(@math.ceil(2.1))     // 3
print(@math.round(2.5))    // 3
print(@math.round(-2.5))   // -3
```

`round` goes to the nearest whole number. A half always goes away from zero, so `2.5` becomes `3` and `-2.5` becomes `-3`.

`sqrt` and `pow` take floats:

```javascript
print(@math.sqrt(16.0))   // 4.0
```

`isNaN` tells whether a float is NaN (not a number, what `0.0 / 0.0` gives). NaN is
the one value that is not equal to itself, so use `isNaN` to check for it. Infinity
is not NaN:

```javascript
print(@math.isNaN(float::nan))   // true
print(float::inf.isNaN())        // false
```

`min` and `max` work with two `int`s or two `float`s, and the two values must have the same type:

```javascript
print(@math.min(3, 9))       // 3
print(@math.max(3, 9))       // 9
print(@math.min(1.5, 0.5))   // 0.5
```

`sign` gives `-1`, `0` or `1`, as an `int`:

```javascript
print(@math.sign(-4))    // -1
print(@math.sign(0))     // 0
print(@math.sign(2.0))   // 1
```

`clamp(x, low, high)` keeps `x` between `low` and `high`:

```javascript
print(@math.clamp(15, 0, 10))         // 10
print(@math.clamp(-3.0, 0.0, 1.0))    // 0.0
```

If `low` is greater than `high`, it is a runtime error.

### Constants

`pi`, `e` and `tau` are functions. Call them with a number of decimal places to round the result, or with nothing for the default:

```javascript
print(@math.pi(4))    // 3.1416
print(@math.pi())     // 3.14
print(@math.e(3))     // 2.718
print(@math.tau(2))   // 6.28
```

The number of decimal places must not be negative. Asking for more places than a float can hold just gives the full value.

### Trigonometry and logarithms

These all take and return floats:

```javascript
print(@math.sin(0.0))                // 0.0
print(@math.atan2(1.0, 1.0))         // 0.7853981633974483
print(@math.log10(1000.0))           // 3.0
print(@math.log2(8.0))               // 3.0
print(@math.toDegrees(@math.pi()))   // 179.9087476710785, because pi() without decimals is 3.14
```

The full list is `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `atan2`, `exp`, `log`, `log2`, `log10`, `toRadians` and `toDegrees`.

### Random numbers

`random()` gives a float from `0` up to, but not including, `1`:

```javascript
let r = @math.random()
print(r >= 0.0 && r < 1.0)   // true
```

### Methods on numbers

Every function above that takes a number first can be called as a method. Note that a negative literal needs parentheses, because `-3.sign()` means `-(3.sign())`:

```javascript
print((-3).abs())    // 3
print(3.min(9))      // 3
print(16.0.sqrt())   // 4.0
print((-3).sign())   // -1
```

## Converting: `@into`

`@into` changes a value from one type to another. It has one function per target type: `string`, `int`, `float`, `f32`, and each sized integer (`i8` to `u64`). Each one can also be called as a method. The sized types are described in [Numbers](numbers.md).

### To string

`@into.string` turns any value into text, using the same format as `print`:

```javascript
print(@into.string(42))       // 42
print(@into.string(2.5))      // 2.5
print(@into.string(true))     // true
print(@into.string([1, 2]))   // [1, 2]
```

The method form works on any value:

```javascript
print(42.string())     // 42
print(true.string())   // true
```

### To int

From a `float`, the decimal part is dropped:

```javascript
print(@into.int(42.9))    // 42
print(@into.int(-42.9))   // -42
```

From a `string`, the text must be a whole number. Spaces around it are allowed:

```javascript
print(@into.int("42"))      // 42
print(@into.int("  7  "))   // 7
print(@into.int("-0"))      // 0
print(@into.int("+5"))      // 5
```

Text that is not a whole number is a runtime error:

```javascript
print(@into.int("abc"))
// runtime error: cannot parse "abc" as int

print(@into.int("4.5"))
// runtime error: cannot parse "4.5" as int
```

A `bool` cannot go to `int`:

```javascript
print(@into.int(true))
// '@into.int' argument #1 expects float, got bool
```

### To float

From an `int`, the value is the same number as a float:

```javascript
print(@into.float(3))         // 3.0
print(@into.float(3) / 2.0)   // 1.5
```

From a `string`, the text can be a decimal or an exponent. Spaces around it are allowed:

```javascript
print(@into.float("3.5"))     // 3.5
print(@into.float("1e3"))     // 1000.0
print(@into.float("  3.5"))   // 3.5
```

Text that is not a number is a runtime error:

```javascript
print(@into.float("x"))
// runtime error: cannot parse "x" as float
```

### Method forms

All three work as methods on their source value:

```javascript
print(3.5.int())          // 3
print("9".int())          // 9
print("2.5".float())      // 2.5
print(5.float().sqrt())   // 2.23606797749979
```

### Edge cases with `int`

Converting `NaN` to `int` gives `0`. A value too large for `int`, including infinity, becomes the largest `int`:

```javascript
print(@into.int(0.0 / 0.0))   // 0
print(@into.int(1.0 / 0.0))   // 9223372036854775807
```

## Limits

- `int` and `float` never convert on their own. Use `@into` to change one into the other.
- `@math` functions do not accept a value of the wrong type. Convert it first.
- `@str.len` and the string positions count characters, not bytes.
