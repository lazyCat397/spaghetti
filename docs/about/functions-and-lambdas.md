# Functions and lambdas

In Spaghetti, a function can be stored in a variable, passed to another function, and returned from one. This page explains the function type, lambdas (anonymous functions), and the most important rule: **a lambda copies the variables it reads**.

## Named functions

A named function is declared with `fn`, a name, typed parameters, and a return type:

```javascript
fn double(x: int) -> int {
    return x * 2
}

print(double(4))   // 8
```

Every parameter needs a type. The return type comes after `->`. Use `-> void` when the function returns nothing.

## Function types

A function type is written as `fn(parameter types) -> return type`:

```javascript
fn(int) -> int             // takes an int, returns an int
fn(int, string) -> bool    // takes an int and a string, returns a bool
fn() -> void               // takes nothing, returns nothing
```

The `-> ...` part is always required, even for `void`. `fn(int)` alone is an error.

You can use a function type anywhere a type is allowed: variables, parameters, return values, struct fields, and inside `array`:

```javascript
let f: fn(int) -> int = double

fn apply(g: fn(int) -> int, value: int) -> int {
    return g(value)
}

let handlers: array<fn() -> string> = [
    fn() -> string { return "a" },
    fn() -> string { return "b" }
]
```

## Named functions as values

Write a function name without parentheses to use it as a value:

```javascript
fn double(x: int) -> int {
    return x * 2
}

let f: fn(int) -> int = double
print(f(4))              // 8

print(apply(double, 10)) // 20
```

Writing the name with parentheses calls the function. Writing it without them gives you the function itself.

## Lambdas

A lambda is a function written inline, without a name:

```javascript
let add = fn(x: int, y: int) -> int {
    return x + y
}

print(add(2, 3))   // 5
```

It works like any other function. It takes typed parameters, and it returns what its `return` says.

A lambda passed directly as an argument:

```javascript
print(apply(fn(n: int) -> int { return n - 1 }, 10))   // 9
```

A lambda that returns another lambda:

```javascript
fn makeAdder(n: int) -> fn(int) -> int {
    return fn(x: int) -> int {
        return x + n
    }
}

print(makeAdder(2)(3))   // 5
```

`makeAdder(2)(3)` calls `makeAdder` with `2`, then calls the result with `3`.

## Calling a function value

When a variable holds a function, call it with parentheses like any function:

```javascript
let f: fn(int) -> int = double
print(f(4))   // 8
```

You can also call a function right where it is written. This is an *immediately invoked* lambda:

```javascript
print(fn() -> int { return 7 }())   // 7
```

At the start of a line, the same form is an expression and not a declaration:

```javascript
fn() {
    print("runs now")
}()
```

To declare a named function, keep the name: `fn name(...)`.

## Capture: a lambda copies what it reads

A lambda can read variables from the function where it was written. When the lambda is created, it **copies** those variables. Changing the variable afterwards does not change the copy:

```javascript
fn main() -> void {
    let total = 10

    let show = fn() -> int {
        return total
    }

    total = 99

    print(show())   // 10, the copy from when the lambda was created
    print(total)    // 99
}

main()
```

Nested lambdas copy values too. A lambda inside a lambda gets the value from the function that originally owns the variable:

```javascript
fn outer() -> void {
    let a = 1

    let mid = fn() -> fn() -> int {
        return fn() -> int { return a + 100 }
    }

    print(mid()())   // 101
}

outer()
```

### A lambda cannot assign to a captured variable

A lambda can read a copy of a variable, but it cannot change it:

```javascript
fn main() -> void {
    let n = 0

    let f = fn() -> void {
        n = n + 1   // type error
    }

    f()
}
```

The compiler reports:

```
a lambda cannot assign 'n': it is a copy of a variable from the function it was written in
```

Inside a lambda, you can still create and change its own variables:

```javascript
let f = fn() -> int {
    let count = 0
    count += 1
    return count
}

print(f())   // 1
```

### Globals are not copied

Variables declared at the top of the file (globals) are not copied. The lambda reads and writes the real global, so it always sees the current value:

```javascript
let total = 10

let g = fn() -> int {
    return total
}

total = 99
print(g())   // 99
```

Because of this, a lambda *can* change a global:

```javascript
let counter = 0

let bump = fn() -> void {
    counter += 1
}

bump()
bump()
print(counter)   // 2
```

Summary of the rule: **locals are copied, globals are shared.** If you want a value to stay fixed, make sure it lives in a local. If you want to change it from a lambda, make it a global.

## Type errors you will see

Calling something that is not a function:

```javascript
let x = 5
print(x(1))
// only functions can be called, and this value is not a function
```

Wrong argument type:

```javascript
let f = fn(x: int) -> int { return x }
print(f("a"))
// argument #1 expects int, got string
```

Wrong number of arguments:

```javascript
let f = fn(x: int) -> int { return x }
print(f(1, 2))
// expects 1 argument(s), got 2
```

A function with a return type that does not return on every path:

```javascript
let f = fn(x: int) -> int {
    if (x > 0) {
        return 1
    }
}
// function '<lambda>' must return int on every path, but a return is missing
```

Assigning a function with the wrong type:

```javascript
let f = fn() -> int { return 1 }
let g: fn(int) -> int = f
// declared type is fn(int) -> int but value is fn() -> int
```

## Printing functions

`print` shows a function as `<function>`:

```javascript
let f = fn(x: int) -> int { return x }
print(f)   // <function>
```

## Methods on value types

Declare a method with `int::methods.add`, `float::methods.add`, `bool::methods.add` or `string::methods.add`. The function is named, and `me` is the value it is called on. `me` is always there and always implicit, with or without other parameters:

```javascript
string::methods.add(fn shout() -> string {
    return "${me}!"
})
int::methods.add(fn double() -> int {
    return me * 2
})

print("hello".shout())   // hello!
print(21.double())       // 42
print("spg".shout().shout())   // spg!!
```

The call is only added at the top level of a file: inside a function or a block it is a compile error, so the list of methods is known before the program runs. Other parameters go after the function name as usual:

```javascript
string::methods.add(fn repeated(times: int) -> string {
    return @str.repeat(me, times)
})
```

`int::methods` lists the names of the methods of the type, as an array of strings: the built-in ones first, in alphabetical order, then the ones you added, in the order you added them. It is a snapshot, so reading it again later gives the same list. `print(int::methods)` shows it.

A method is visible to the files that attach its module only when it is declared with `public` in front of the statement: `public string::methods.add(fn ...)`. A method without `public` can be called only from its own file. Two modules may each keep a private method with the same name.

Value types always return a new value: the method cannot change the original. A method's name cannot be the name of a built-in method of that type (`len` on `string`, for example), and declaring one is a compile error.

## Higher-order helpers

The `@array` and `@map` namespaces take functions for `map`, `filter` and `reduce`. They are covered on the [collections page](collections.md#transforming-arrays).

## Generic functions

A function can work on more than one type. Name its type parameters after the function, and give each one a bound: the types it accepts.

```javascript
fn dobro<T: int or float>(n: T) -> T {
    return n + n
}

print(dobro(3), dobro(2.5))   // 6 5.0
```

`T` is decided by the arguments. Each type used gets its own copy of the function, checked for that type, so `dobro(3)` runs with `int` and `dobro(2.5)` with `float`. Nothing is converted behind your back: the body sees exactly the types the call gave it.

### The Numeric bound

`Numeric` means every integer and float type: `int`, `i8`, `i16`, `i32`, `u8`, `u16`, `u32`, `u64`, `float` and `f32`.

```javascript
fn add<T: Numeric>(n: T) -> T {
    return n + T.from(1)
}

print(add(3.4), add(5), add(-3.2))   // 4.4 6 -2.2

let u: u8 = 7
print(add(u))                          // 8
```

Inside a generic, `T.from(1)` turns a literal into the type `T` exactly. An integer literal becomes a `float` only when the `float` holds it exactly (up to 2^53, and up to 2^24 for `f32`). `T.from(1.5)` for an `int` `T` is an error. `T.from` takes literals only: to convert a variable, use `@into`.

Without `T.from`, `1 + 1.0` is still an error, as it is everywhere else in the language.

### Type parameters without a bound

A parameter with no bound accepts any type. The body is checked for each type it is called with, so it works for whatever the body allows:

```javascript
fn soma2<T>(a: T, b: T) -> T {
    return a + b
}

print(soma2(1, 2), soma2("a", "b"))   // 3 ab
```

A generic with no bound is checked only where it is called. A generic whose parameters all have a bound is also checked for every type the bound allows, so an error in its body shows up even without a call.

### Arrays and optionals

A type parameter can appear inside `array<T>` and `T?`. It is inferred from the value passed in:

```javascript
fn primeiro<T>(xs: array<T>) -> T? {
    if (@array.len(xs) == 0) {
        return none
    }
    return xs[0]
}

print(primeiro([1, 2, 3]))   // 1
```

### What is not in generic functions yet

- **Methods.** A method of a struct cannot have type parameters. This is a compile error, not a silent drop.
- **Explicit types at the call.** `add<int>(3)` is not written; the type always comes from the arguments.
- **Generic structs and enums.** Only functions take type parameters.
- **Generic as a value.** `let f = add` is an error: a generic needs its arguments to be a value. Call it, or write the function you need.
- **Nesting.** A generic that keeps calling itself with a bigger type (such as wrapping its argument in an array each time) is an error after 32 levels.

## Limits

- Functions cannot be compared with `==` or `!=` yet.
- JSON cannot hold a function. Trying to convert one with `@json.stringify` reports `@json.stringify: a function has no JSON form`.
- Lambdas capture by value only. There is no way to share a local between a lambda and the function that created it. Use a global for that.
