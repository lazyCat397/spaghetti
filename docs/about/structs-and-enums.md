# Structs and enums

A **struct** groups named fields into one value. An **enum** is a type with a fixed list of named options. This page covers both, plus methods on structs, associated functions, namespaces, and values on enum options.

## Structs

Declare a struct with its fields. Separate fields with commas:

```javascript
struct Point {
    x: int,
    y: int
}
```

Create one with a struct literal. Every field must be given, and each value must match its type:

```javascript
let p = Point { x: 3, y: 4 }
print(p.x)   // 3
```

Read and write fields with a dot:

```javascript
p.x = 10
p.y += 1
print(p.x, p.y)   // 10 5
```

Structs can hold other structs, and you can chain dots as deep as you need:

```javascript
struct Box {
    point: Point,
    label: string
}

let b = Box {
    point: Point { x: 1, y: 2 },
    label: "corner"
}

b.point.x = 50
print(b.point.x)   // 50
```

A field can also be optional. Use `none` for a missing link, which is how you build a linked list:

```javascript
struct Node {
    value: int,
    next: Node?
}

let n = Node { value: 1, next: none }
```

### Destructuring fields

`let { x, y } = point` reads fields by name:

```javascript
let { x, y } = Point { x: 3, y: 4 }
print(x * y)            // 12
```

### Structs are shared, not copied

A struct value is a reference. Passing it to a function, or assigning it to another variable, does not copy it. Both names point to the same struct:

```javascript
fn move(q: Point) -> void {
    q.x += 100
}

move(p)
print(p.x)   // 110, the function changed the same struct

let copy = p
copy.y = 0
print(p.y)   // 0, `copy` is the same struct as `p`
```

If you need an independent struct, use `.copy()`:

```javascript
let other = p.copy()
```

### Comparing structs

`==` and `!=` compare the fields. Two structs with the same field values are equal, even if they are different objects:

```javascript
let a = Point { x: 1, y: 2 }
let b = Point { x: 1, y: 2 }

print(a == b)                       // true
print(a == Point { x: 9, y: 2 })    // false
```

### Printing a struct

`print` shows the struct name and its fields:

```javascript
print(p)   // Point { x: 110, y: 0 }
```

## Methods

A method is a function written inside the struct body. Call it with a dot:

```javascript
struct Counter {
    count: int,

    fn tick() -> int {
        me.count += 1
        return me.count
    }
}

let c = Counter { count: 0 }
print(c.tick())   // 1
print(c.tick())   // 2
```

Inside a method, `me` is the struct the method was called on. You do not write `me` in the parameter list. It is always there, and it always comes first.

Methods can take other parameters, and they can return the struct itself:

```javascript
struct Counter {
    count: int,

    fn addMany(n: int) -> void {
        me.count += n
    }

    fn reset() -> Counter {
        me.count = 0
        return me
    }
}

let c = Counter { count: 0 }
c.addMany(5)
print(c.count)           // 5
print(c.reset().count)   // 0
```

Methods can build new structs of their own type:

```javascript
struct Rect {
    w: int,
    h: int,

    fn area() -> int {
        return me.w * me.h
    }

    fn scaled(k: int) -> Rect {
        return Rect { w: me.w * k, h: me.h * k }
    }
}

let r = Rect { w: 2, h: 3 }
print(r.area())            // 6
print(r.scaled(2).area())  // 24
```

Inside the struct body, fields are read as `me.field`. A bare name like `w` does not refer to the field.

### Fields and methods are different

A field is read without parentheses. A method needs parentheses:

```javascript
print(r.w)        // field: 2
print(r.area())   // method: 6
```

Calling a field or reading a method without calling it are both errors:

```javascript
p.x()
// 'x' is a field of struct 'P', not a method (fields cannot be called)

p.get
// 'get' is a method of struct 'P', not a field; call it with '.get()'
```

## Associated functions

A method needs a value to be called on. An **associated function** belongs to the type itself, so you call it on the type name. Write it at the top level of the file, with the type name before the function name, and leave out `me`:

```javascript
fn Point.origin() -> Point {
    return Point { x: 0, y: 0 }
}

let o = Point.origin()
print(o)   // Point { x: 0, y: 0 }
```

The same works for enums. Associated functions are the closest thing Spaghetti has to static methods:

```javascript
enum Light { Red, Yellow, Green }

fn Light.first() -> Light {
    return Light.Red
}

print(Light.first())   // Red
```

## Namespaces

A **namespace** groups functions and static values under one name. It has no instances, so there is no `me` and nothing to build. Call its members with the name in front:

```javascript
public namespace som {
    let calls;                  // a static: starts as int 0, like any `let` without a value

    fn add(a: int, b: int) -> int {
        calls += 1               // inside, a static is written by its bare name
        return a + b
    }

    fn double(a: int) -> int {
        return add(a, a)         // a sibling function is called by its bare name too
    }

    fn count() -> int {
        return calls
    }
}

print(som.add(1, 2))             // 3
print(som.double(5))             // 10
print(som.count())               // 2
print(som.calls)                 // reading a static from outside is fine
```

Statics can be read from outside the namespace but not written: `som.calls = 0` is a compile error. Only the namespace's own functions change them. A `const` static works the same way, and cannot change even inside.

`public namespace` shares the namespace itself. Its members stay private to the file unless each one is marked `public`, so a helper inside the namespace can be left out of the public API:

```javascript
public namespace tool {
    let secret;                  // private: other files cannot read it

    public fn shown(a: int) -> int {
        secret += 1
        return hidden(a)         // a private sibling is still callable inside the file
    }

    fn hidden(a: int) -> int {   // private: other files cannot call tool.hidden()
        return a * 10
    }
}
```

From another file, `use (tool) from "tool.spg"` gives you `tool.shown(4)`, and `tool.hidden(4)` or `tool.secret` are compile errors. A namespace is not a value: you cannot store `som` in a variable or pass it to a function. Its functions are values, though: `let f = som.add` works, and so does passing `som.add` to another function.

Namespaces are for grouping. If you need many instances with their own data, use a `struct` instead.

Limits, for now:

- Declare the namespace before the code that reads its statics, the same way you declare a `let` before using it.
- There is no nesting: a namespace cannot contain another namespace.
- A namespace can only be declared at the top level of a file.
- Destructuring (`let (a, b)`) is not allowed inside a namespace.

## Struct errors

Unknown field:

```javascript
let p = P { x: 1 }
print(p.z)
// struct 'P' has no field 'z'
```

Missing field in the literal:

```javascript
struct P { x: int, y: int }
let p = P { x: 1 }
// struct literal for 'P' is missing field(s): y
```

Extra field in the literal:

```javascript
struct P { x: int }
let p = P { x: 1, y: 2 }
// struct 'P' has no field 'y'
```

Wrong value type:

```javascript
struct P { x: int }
let p = P { x: "a" }
// field 'x' of struct 'P' expects int, found string
```

## Enums

An enum lists the options a value can take. Each option is called a variant:

```javascript
enum Color { Red, Green, Blue }

let c = Color.Green
print(c)   // Green
```

Use the enum name, a dot, and the variant. Enums compare with `==` and `!=`, and only with the same enum:

```javascript
print(c == Color.Green)   // true
print(c != Color.Red)     // true
```

Printing an enum shows the variant name, also inside arrays and structs:

```javascript
let arr = [Color.Red, Color.Blue]
print(arr)   // [Red, Blue]

struct Light {
    state: Color
}

let l = Light { state: Color.Blue }
print(l)     // Light { state: Blue }
```

Enums are their own type. They are not `int`, and you cannot use them as numbers:

```javascript
print(Color.Red < Color.Green)   // error: operator Lt not supported for Color and Color
let n: int = Color.Red           // error: declared type is int but value is Color
```

### Matching on an enum

`when` picks the branch for each variant. An enum with every variant listed does not need an `else`:

```javascript
fn label(c: Color) -> string {
    when (c) {
        Color.Red {
            return "stop"
        }
        Color.Green {
            return "go"
        }
        Color.Blue {
            return "cool"
        }
    }
}

print(label(Color.Green))   // go
```

If you leave a variant out, add an `else` branch. Without it, the compiler asks for every variant.

### Looping over an enum

`for` runs once for each variant, in the order they were declared:

```javascript
enum Color { Red, Green, Blue }

for (each in Color) {
    print(each)
}
// Red
// Green
// Blue
```

### Values on variants

A variant can carry a value. Write `= value` after its name:

```javascript
enum Level { Low, Mid = 10, High }

print(Level.Low.value)    // 0
print(Level.Mid.value)    // 10
print(Level.High.value)   // 11
```

Without `=`, a variant takes the previous value plus one. The first one starts at `0`. So `Low` is `0`, `Mid` is `10`, and `High` is `11`.

Values can be `int`, `float`, `string` or `bool`. All variants of the same enum must use the same type:

```javascript
enum Status { Ok = "ok", Failed = "failed" }
print(Status.Failed.value)   // failed

enum Ratio { Half = 0.5, Full = 1.0 }
print(Ratio.Half.value + Ratio.Full.value)   // 1.5
```

Read the value with `.value`. It has the type of the enum's values, so `Ratio.Half.value` is a `float`. You can also loop and read the value:

```javascript
enum Level { Low, Mid = 10 }

let total = 0

for (lv in Level) {
    total += lv.value
}

print(total)   // 10
```

Comparing two enums compares the variant, not the value:

```javascript
enum Ns { one = 1, two = 2, three = 2 }

print(Ns.two == Ns.three)               // false, different variants
print(Ns.two.value == Ns.three.value)   // true, same value
```

### Enum errors

Unknown variant. The error lists the valid ones:

```javascript
enum Color { Red, Green }
let c = Color.Blue
// enum 'Color' has no variant 'Blue' (variants: Red, Green)
```

Repeated variant name:

```javascript
enum Color { Red, Red }
// enum 'Color' already has a variant named 'Red'
```

An enum with no variants:

```javascript
enum Color { }
// enum 'Color' needs at least one variant
```

Mixing value types:

```javascript
enum Plain { A, B = "x" }
// enum 'Plain': variant 'B' has a string value but the earlier variants are int
```

The enum name on its own is not a value:

```javascript
let x = Color
// 'Color' is an enum, not a value; pick a variant ('Color.<Variant>') or loop over it ('for (x in Color)')
```

Enums have no methods, but they can have associated functions. Calling a method on an enum value is an error:

```javascript
let c = Color.Red
print(c.len())
// type Color has no methods (cannot call '.len()' on it)
```

Note that `.value` is the way to get the stored value. `@into.int(c)` does not work on an enum.

## Limits

- Enums have no methods of their own. Use associated functions for helpers that belong to the type.
- Methods live inside the struct body. Associated functions live at the top level.
- Two variants can share a value, but not a name.
