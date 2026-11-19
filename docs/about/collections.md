# Collections: arrays and maps

An **array** is an ordered list of values of the same type. A **map** stores pairs of key and value, where each key appears once. This page covers both, their methods, and their errors.

## Arrays

Write the items between brackets. All items must have the same type:

```javascript
let nums: array<int> = [3, 1, 2]
print(nums)   // [3, 1, 2]
```

The type is `array<T>`, where `T` is the type of the items. You can nest arrays:

```javascript
let grid: array<array<int>> = [[1, 2], [3, 4]]
print(grid[1][0])   // 3
```

An empty array needs its type written out:

```javascript
let names: array<string> = []
```

Always write the type on an empty array, as above. Leaving it out is not a safe shortcut (see the limits at the end of this page).

### Reading and writing items

Indexes start at `0`:

```javascript
let nums: array<int> = [3, 1, 2]
print(nums[0])   // 3

nums[1] = 10     // replace an item
nums[2] += 5     // compound assignment works too
print(nums)      // [3, 10, 7]
```

Reading an index that does not exist is a runtime error:

```javascript
let a: array<int> = [1, 2]
print(a[5])
// runtime error: index out of bounds: 5 (array has 2 element(s))
```

Index values must be `int`:

```javascript
print(a["0"])
// array index must be int, found string
```

### Destructuring

`let (a, b) = items` reads the first items by position into names:

```javascript
let (first, second) = [10, 20, 30]
print(first + second)   // 30
```

If the array is shorter than the names, the program stops with an index error at
that line. Destructuring works with `let` and `public let` only.

### Looping

`for` runs the body once for each item:

```javascript
let total = 0

for (n in [1, 2, 3]) {
    total += n
}

print(total)   // 6
```

When you also need the position, write two names. The first is the index, the second is the item:

```javascript
let names = ["ana", "bia", "caio"]

for (i, name in names) {
    print("${i}: ${name}")
}
// 0: ana
// 1: bia
// 2: caio
```

Two names work only on arrays. Use `for k, v in` on a map instead (see [Looping over a map](#looping-over-a-map)).

### Methods

Array functions live in the `@array` namespace. Most of them can also be called as methods on the array itself, with the array first: `nums.push(7)` is the same as `@array.push(nums, 7)`.

Adding and removing:

```javascript
let nums: array<int> = [3, 10, 7]

nums.push(7)                // add to the end
print(nums.pop(), nums)     // 7 [3, 10, 7]

@array.insert(nums, 1, 99)  // insert at index 1
print(nums)                 // [3, 99, 10, 7]

@array.remove(nums, 0)      // remove the item at index 0
print(nums)                 // [99, 10, 7]

@array.clear(nums)          // empty the array
print(nums)                 // []
```

`pop` on an empty array is a runtime error:

```javascript
let a: array<int> = []
a.pop()
// runtime error: @array.pop on an empty array
```

Searching and measuring:

```javascript
let nums: array<int> = [3, 10, 7]

print(nums.len())                   // 3
print(@array.contains(nums, 10))    // true
print(nums.indexOf(3))              // 0
print(@array.slice(nums, 0, 2))     // [3, 10]
```

Getting the first and the last item. Both give an optional, so the result is `none` for an empty array:

```javascript
let nums: array<int> = [3, 10, 7]
let empty: array<int> = []

print(@array.first(nums))     // 3
print(@array.last(nums))      // 7
print(@array.first(empty))    // none
```

Ordering and combining:

```javascript
let nums: array<int> = [3, 1, 2]
@array.sort(nums)
print(nums)                                  // [1, 2, 3]

let back: array<int> = [1, 2, 3]
print(back.reverse())                        // [3, 2, 1]
print(@array.concat([1, 2], [3]))            // [1, 2, 3]
print(@array.sum([1, 2, 3]))                 // 6
print(@array.join(["a", "b", "c"], ", "))    // a, b, c
```

`sort` and `reverse` change the array in place, and they also return it. `concat` returns a new array and does not change its inputs.

Building arrays:

```javascript
print(@array.range(1, 4))        // [1, 2, 3]
print(@array.range(0, 10, 3))    // [0, 3, 6, 9]
print(@array.fill(3, "x"))       // [x, x, x]
```

`range` ends before the stop value. `range(0, 10, 3)` stops before `10`, and the third argument is the step.

### Transforming arrays

`map`, `filter` and `reduce` take a function and apply it to the items. The function is written inline with `fn`, and its types are written out, like any other function:

```javascript
let scores: array<int> = [10, 20, 30]

let doubled = @array.map(scores, fn(x: int) -> int {
    return x * 2
})

let big = @array.filter(scores, fn(x: int) -> bool {
    return x > 15
})

let total = @array.reduce(scores, fn(acc: int, x: int) -> int {
    return acc + x
}, 0)

print(doubled, big, total)
// [20, 40, 60] [20, 30] 60
```

- `map` returns a new array with the result of the function for each item.
- `filter` returns a new array with the items for which the function returns `true`.
- `reduce` combines the items into one value. It starts from the last argument, here `0`, and the function gets the running total first and the item second.

The checker reads these types, so a function with the wrong parameter or return type is a type error, the same as anywhere else.

### Arrays are shared

Like structs, arrays are references. Assigning an array to another variable does not copy it:

```javascript
let a: array<int> = [1, 2]
let b = a

b[0] = 99
print(a[0])   // 99
```

To make an independent copy, use `.copy()`:

```javascript
let copy = a.copy()
```

### Equality

Two arrays are equal when their items are equal, in the same order:

```javascript
print([1, 2] == [1, 2])   // true
print([1, 2] == [1])      // false
```

### Membership

`in` checks whether an item is in an array. It is the same as `@array.contains`:

```javascript
let names = ["ana", "bia"]

print("bia" in names)   // true
print("bob" in names)   // false
```

### Type errors

Mixed item types:

```javascript
let a = [1, 2.5]
// all items of an array must have the same type: expected int, found float
```

Adding the wrong type:

```javascript
let a = [1]
a.push("x")
// '@array.push' argument #1 expects int, got string
```

Passing the wrong array type:

```javascript
print(@array.sum(["a"]))
// '@array.sum' argument #1 expects array<int>, got array<string>
```

## Maps

A map pairs keys with values. The type is `map<K, V>`:

```javascript
let ages: map<string, int> = ["ana": 30, "bia": 25]

print(ages)          // [ana: 30, bia: 25]
print(ages["ana"])   // 30
```

An empty map is written `[:]`, and it needs its type:

```javascript
let empty: map<int, string> = [:]
print(empty)   // [:]
```

Writing `[:]` without a type is an error:

```javascript
let m = [:]
// cannot tell what '[:]' holds; write the type, e.g. 'let m: map<string, int> = [:]'
```

### Keys

Keys can be `int`, `string`, `bool`, or an enum. Keys cannot be `float`, arrays, or structs:

```javascript
let m: map<float, int> = [1.5: 1]
// map keys must be int, string, bool or an enum, found float
```

Values can be any type, including arrays, maps and structs:

```javascript
let groups: map<string, array<int>> = [
    "odd": [1, 3],
    "even": [2]
]

groups["odd"].push(5)
print(groups)   // [odd: [1, 3, 5], even: [2]]
```

### Writing and reading

Assigning to a key inserts it, or replaces the value if the key already exists:

```javascript
let ages: map<string, int> = ["ana": 30]

ages["bia"] = 25   // insert
ages["ana"] = 31   // replace
print(ages)        // [ana: 31, bia: 25]
```

Compound assignment works on existing keys:

```javascript
ages["bia"] += 1
```

Reading a key that does not exist is a runtime error. Check with `@map.has` first, or use `@map.get` with a fallback:

```javascript
let m: map<string, int> = ["a": 1]
print(m["zz"])
// runtime error: key not found in map: "zz" (use @map.has(m, k) or @map.get(m, k, fallback))
```

Counting words is the common case, so write it with `has`:

```javascript
let counts: map<string, int> = [:]

for (w in ["a", "b", "a"]) {
    if (@map.has(counts, w)) {
        counts[w] += 1
    } else {
        counts[w] = 1
    }
}

print(counts)   // [a: 2, b: 1]
```

Duplicate keys in a literal are allowed. The last one wins:

```javascript
let m: map<string, int> = ["a": 1, "a": 2]
print(m)   // [a: 2]
```

### Methods

Map functions live in the `@map` namespace. As with arrays, they can also be called as methods on the map.

```javascript
let ages: map<string, int> = ["ana": 31, "bia": 26]

print(@map.has(ages, "bia"), @map.has(ages, "zz"))   // true false
print(@map.get(ages, "zz", 0))                       // 0
print(ages.len(), ages.has("ana"))                   // 2 true
print(@map.keys(ages))                               // [ana, bia]
print(@map.values(ages))                             // [31, 26]
```

`set` adds a key, or replaces its value if the key is already there. It does the same as `m[key] = value`, and you can use whichever reads better:

```javascript
let ages: map<string, int> = ["ana": 30]

@map.set(ages, "bia", 25)     // adds bia
@map.set(ages, "ana", 31)     // replaces ana
print(ages)                   // [ana: 31, bia: 25]
```

`remove` returns `true` if the key was there, and `false` if it was not:

```javascript
let m: map<string, int> = ["a": 1]
print(@map.remove(m, "a"), @map.remove(m, "a"))   // true false
```

Find the keys that have a given value with `keysOf`. It returns an array, empty if nothing matches:

```javascript
print(@map.keysOf(ages, 31))   // [ana]
```

Filter and transform a map with functions that receive the key and the value:

```javascript
let prices: map<string, int> = ["bread": 5, "milk": 8]

let cheap = @map.filter(prices, fn(k: string, v: int) -> bool {
    return v < 6
})

let doubled = @map.map(prices, fn(k: string, v: int) -> int {
    return v * 2
})

print(cheap)     // [bread: 5]
print(doubled)   // [bread: 10, milk: 16]
```

`filter` keeps the pairs for which the function returns `true`. `map` keeps the keys and replaces each value with the result of the function.

### Looping over a map

Use two names to get the key and the value together. Keys come in insertion order:

```javascript
let ages: map<string, int> = ["ana": 31, "bia": 25]

for (name, age in ages) {
    print("${name} is ${age}")
}
// ana is 31
// bia is 25
```

You can also loop over the keys or the values alone:

```javascript
for (k in @map.keys(counts)) {
    print(k, counts[k])
}
// a 2
// b 1
```

With a single name, `for (k in map)` is an error. Use `@map.keys(map)` or the two-name form.

Maps keep their insertion order. `print`, `keys`, `values` and the loops all follow it.

### Maps are shared

Like arrays and structs, a map is a reference:

```javascript
let a: map<int, int> = [1: 10]
let b = a

b[1] = 99
print(a[1])   // 99
```

### Type errors

Wrong value type in a literal:

```javascript
let m: map<string, int> = ["a": "x"]
// declared type is map<string, int> but value is map<string, string>
```

Using `len` on a map. Use `@map.len` or the method instead:

```javascript
print(len(m))
// 'len' argument #1 expects string, got map<string, int>
```

`for` with a single name over a map:

```javascript
for (k in m) {
    print(k)
}
// type map<string, int> is not iterable
```

## Limits

- Writing the type on an empty array `[]` is required. The compiler currently accepts `let a = []` without one, but the array then has no type, so the items you add later are not checked. Always write the type.
- Map keys cannot be `float`.
- `remove` on a map is slower than other map operations, because it has to reorder what comes after it. Use it sparingly on very large maps.
