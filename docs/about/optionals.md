# Optional types: `T?`, `none` and `??`

Sometimes a value may be missing. A function might not find what it looks for, or a variable might not be set yet. In Spaghetti, a type with a `?` at the end means "this value is a `T`, or it is `none`".

```javascript
let name: string? = none
print(name ?? "nobody")   // nobody
```

## Declaring an optional

Add `?` after any type:

```javascript
let age: int? = none
let score: int? = 10
```

Optional types work everywhere a type works: variables, parameters, return types, struct fields, and inside `array` and `map`:

```javascript
fn find(items: array<int>, target: int) -> int? {
    let i = 0

    while (i < @array.len(items)) {
        if (items[i] == target) {
            return i
        }
        i += 1
    }

    return none
}

struct Node {
    value: int,
    next: Node?
}

let values: array<int?> = [1, none, 3]
```

`none` alone has no type, so you always need to say which optional type it is. `let x = none` is an error. Write `let x: int? = none`.

An `int` never becomes `none`. A value of type `int` can go into an `int?`, but not the other way around:

```javascript
let n: int = 5
let m: int? = n    // fine
let k: int = m     // error: int? is not int
```

## Checking for `none`

Before you use an optional as its plain type, check it:

```javascript
let age: int? = 30

if (age != none) {
    print(age + 1)   // age is an int here
}
```

Inside the block, the compiler knows `age` is an `int`, so the arithmetic is allowed. The same works with `==` and `else`:

```javascript
if (age == none) {
    print("unknown")
} else {
    print(age + 1)
}
```

It works in `while` too:

```javascript
let current: Node? = head

while (current != none) {
    total += current.value
    current = current.next
}
```

Assigning to the variable ends the check. After `current = current.next`, `current` is a `Node?` again, so the loop condition checks it again.

### Early exit

If a branch always leaves the block (with `return`, `throw`, `break` or `continue`), the rest of the block knows the check passed:

```javascript
fn double(x: int?) -> int {
    if (x == none) {
        return -1
    }

    return x * 2   // x is an int from here on
}
```

The same goes for `||`. If either side is `none`, you stop early:

```javascript
fn add(a: int?, b: int?) -> int {
    if (a == none || b == none) {
        return 0
    }

    return a + b
}
```

### Limits of the check

The check works on plain variables only. It does not work on fields or array items:

```javascript
// this does NOT compile
if (node.next != none) {
    print(node.next.value)
}

// do this instead
let next = node.next

if (next != none) {
    print(next.value)
}
```

## Default values with `??`

`a ?? b` gives `a` when it has a value, and `b` when `a` is `none`:

```javascript
let port: int? = none
let real = port ?? 8080   // 8080
```

The left side must be optional. The right side is only evaluated when the left side is `none`, so it is safe to put a call there:

```javascript
let cache: int? = 10
let value = cache ?? compute()   // compute() does not run here
```

`??` chains from left to right, and it has the lowest precedence of the binary operators:

```javascript
let x: int? = none
let y: int? = none
let z = x ?? y ?? 0   // 0

let flag: bool? = none
let other: bool = false
let w = flag ?? other || true   // same as: flag ?? (other || true)
```

If the right side is also optional, the result stays optional:

```javascript
let first: int? = find(items, 3)
let second: int? = find(items, 4)
let either = first ?? second   // int?
```

## Comparing optionals

You can compare with `none`, and you can compare an optional with a plain value of its inner type:

```javascript
let x: int? = 3

print(x == none)   // false
print(x == 3)      // true
print(x != 4)      // true
```

Two optionals of the same type can be compared too:

```javascript
let a: int? = none
let b: int? = none

print(a == b)   // true
```

Comparing `int?` with `string?` is an error, because the types are different.

## Printing

`none` prints as `none`, both with `print` and inside a string:

```javascript
let x: int? = none

print(x)                // none
print("value: ${x}")    // value: none
```

In JSON, `none` becomes `null`.

## Common errors

Using an optional as if it had a value. The compiler adds a hint to the message:

```javascript
let x: int? = 5
print(x + 1)
// type error ... operator Add not supported for int? and int (a T? may be none: check it with `!= none` first, or give it a default with `??`)
```

Comparing two optionals of different types:

```javascript
let x: int? = 3
let y: string? = "a"
print(x == y)
// type error ... operator Eq not supported for int? and string? (...)
```

Forgetting the type on `none`:

```javascript
let z = none
// cannot tell which type 'none' belongs to; write it, e.g. 'let x: int? = none'
```

`??` on a value that is not optional:

```javascript
print(5 ?? 3)
// type error ... '??' needs an optional value on its left (like 'int?'), but found int
```

`void?` has no meaning, because `void` holds no value. This one is a parse error:

```javascript
let a: void? = none
// parse error ... expected a type that can hold a value (void? makes no sense)
```

## A note on `none` and globals

Narrowing works on variables. If a global variable is narrowed in one function, and another function sets it to `none` while the first one is still running, you get a runtime error:

```javascript
let g: int? = 5

fn clear() -> void {
    g = none
}

fn use() -> int {
    if (g == none) {
        return 0
    }

    clear()
    return g + 1   // runtime error: unexpected none
}
```

The error can be caught with `try`/`catch`. To avoid it, copy the global into a local before you check it:

```javascript
fn use() -> int {
    let current = g

    if (current == none) {
        return 0
    }

    clear()
    return current + 1
}
```
