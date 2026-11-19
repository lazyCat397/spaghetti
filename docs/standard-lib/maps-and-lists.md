# Maps and lists: `@array` and `@map`

This page is the reference for the functions in the `@array` and `@map` namespaces. For the language rules about arrays and maps (literals, types, `for`, sharing), see [collections.md](../about/collections.md).

Every function below can also be called as a method on its first argument. `arr.push(x)` is the same as `@array.push(arr, x)`, and `m.has(k)` is the same as `@map.has(m, k)`. The examples use the function form.

Functions that change the collection say so. The others return a new value and leave the original as it is.

## `@array`

### Size and access

**`@array.len(a)`** returns the number of items as an `int`.

```javascript
print(@array.len([1, 2, 3]))   // 3
```

**`@array.contains(a, x)`** returns `true` if `x` is in `a`.

```javascript
print(@array.contains([1, 2], 2))   // true
```

**`@array.indexOf(a, x)`** returns the position of the first item equal to `x`, or `-1` if there is none.

```javascript
print(@array.indexOf(["b", "a"], "a"))   // 1
print(@array.indexOf([1, 2], 9))         // -1
```

**`@array.slice(a, start, end)`** returns a new array with the items from `start` up to `end`. The `end` position is not included. Both positions must be inside the array, or it is a runtime error.

```javascript
print(@array.slice([3, 10, 7], 0, 2))   // [3, 10]
print(@array.slice([3, 10, 7], 1, 1))   // []
```

**`@array.first(a)`** and **`@array.last(a)`** return the first and the last item as an optional. For an empty array, both return `none`.

```javascript
print(@array.first([3, 10, 7]))   // 3
print(@array.last([3, 10, 7]))    // 7
print(@array.first([]))           // none
```

### Adding and removing

**`@array.push(a, x)`** adds `x` to the end. It changes `a` and returns nothing. `x` must have the type of the items.

```javascript
let a: array<int> = [1]
@array.push(a, 2)
print(a)   // [1, 2]
```

**`@array.pop(a)`** removes the last item and returns it. On an empty array it is a runtime error.

```javascript
let a: array<int> = [1, 2]
print(@array.pop(a), a)   // 2 [1]
```

**`@array.insert(a, i, x)`** puts `x` at position `i`, moving the items after it one step forward. `i` can be from `0` up to the length of `a`, inclusive. Inserting at the length adds to the end. It changes `a` and returns nothing.

```javascript
let a: array<int> = [1]
@array.insert(a, 0, 5)
@array.insert(a, 2, 9)
print(a)   // [5, 1, 9]
```

**`@array.remove(a, i)`** removes the item at position `i` and returns it. It changes `a`. A position outside the array is a runtime error.

```javascript
let a: array<int> = [1, 2, 3]
print(@array.remove(a, 1), a)   // 2 [1, 3]
```

**`@array.clear(a)`** removes every item. It changes `a` and returns nothing.

```javascript
let a: array<int> = [1, 2]
@array.clear(a)
print(a)   // []
```

### Ordering and combining

**`@array.sort(a)`** sorts `a` from smallest to largest. It works with `int`, `float`, `bool`, `string`, and the sized numbers (`i8` to `u64`, `f32`). An array of `u64` is ordered as unsigned numbers. It changes `a` and also returns it.

```javascript
let a: array<int> = [3, 1, 2]
@array.sort(a)
print(a)   // [1, 2, 3]
```

**`@array.reverse(a)`** reverses the order. It changes `a` and also returns it.

```javascript
let a: array<int> = [1, 2, 3]
print(@array.reverse(a))   // [3, 2, 1]
```

**`@array.concat(a, b)`** returns a new array with the items of `a` followed by the items of `b`. Neither input changes. Both must have the same item type.

```javascript
print(@array.concat([1, 2], [3]))   // [1, 2, 3]
```

**`@array.join(a, sep)`** joins an array of strings into one string, with `sep` between the items. `a` must be `array<string>`.

```javascript
print(@array.join(["a", "b", "c"], ", "))   // a, b, c
```

**`@array.sum(a)`** adds up the items and returns the total, with the same type as the items. `a` must be `array<int>` or `array<float>`. An empty array gives `0`.

```javascript
print(@array.sum([1, 2, 3]))    // 6
print(@array.sum([1.5, 2.0]))   // 3.5
```

### Transforming arrays

These take a function. The function is written inline with `fn`, and its parameter and return types must match the items:

**`@array.map(a, fn(x: T) -> U)`** returns a new array with the result of the function for each item. The result is `U[]`.

**`@array.filter(a, fn(x: T) -> bool)`** returns a new array with the items for which the function returns `true`.

**`@array.reduce(a, fn(acc: A, x: T) -> A, init)`** combines the items into one value. It starts with `init`, and the function receives the running value first and the item second.

```javascript
let nums: array<int> = [1, 2, 3, 4]

let squares = @array.map(nums, fn(x: int) -> int {
    return x * x
})

let evens = @array.filter(nums, fn(x: int) -> bool {
    return x % 2 == 0
})

let product = @array.reduce(nums, fn(acc: int, x: int) -> int {
    return acc * x
}, 1)

print(squares, evens, product)
// [1, 4, 9, 16] [2, 4] 24
```

See [collections.md](../about/collections.md#transforming-arrays) for more about these functions.

### Building arrays

**`@array.range(start, stop)`** returns the integers from `start` up to `stop`, not including `stop`. It counts up when `start` is smaller, and down when it is larger.

**`@array.range(start, stop, step)`** does the same with a step between values. The step cannot be `0`.

```javascript
print(@array.range(1, 4))       // [1, 2, 3]
print(@array.range(0, 10, 3))   // [0, 3, 6, 9]
print(@array.range(10, 0, -3))  // [10, 7, 4, 1]
print(@array.range(5, 1))       // []
```

**`@array.fill(n, x)`** returns an array with `n` copies of `x`. `x` must be an `int`, `float`, `bool` or `string`. Arrays and structs are not allowed, because every copy would point to the same value.

```javascript
print(@array.fill(3, "x"))   // [x, x, x]
print(@array.fill(0, 5))     // []
```

### Errors

Wrong item type when adding:

```javascript
let a = [1]
@array.push(a, "x")
// '@array.push' argument #2 expects int, got string
```

Wrong array type for a function that needs strings:

```javascript
print(@array.join([1, 2], ","))
// '@array.join' argument #1 expects array<string>, got array<int>
```

Mixed item types in a literal:

```javascript
let a = [1, 2.5]
// all items of an array must have the same type: expected int, found float
```

Position outside the array:

```javascript
let a = [1]
@array.remove(a, 3)
// runtime error: index out of bounds: 3 (array has 1 element(s))
```

Zero step in `range`:

```javascript
print(@array.range(0, 3, 0))
// runtime error: range step cannot be zero
```

## `@map`

### Checking and reading

**`@map.has(m, k)`** returns `true` if the key `k` is in `m`.

```javascript
let ages: map<string, int> = ["ana": 30]

print(@map.has(ages, "ana"))   // true
print(@map.has(ages, "zz"))    // false
```

**`@map.get(m, k, fallback)`** returns the value for `k`, or `fallback` if the key is not there. The fallback must have the value type of the map.

```javascript
print(@map.get(ages, "ana", 0))   // 30
print(@map.get(ages, "zz", -1))   // -1
```

Reading a missing key with `m[k]` is a runtime error, so use `@map.get` or check with `@map.has` first.

**`@map.len(m)`** returns the number of entries.

```javascript
print(@map.len(ages))   // 1
```

### Removing

**`@map.remove(m, k)`** removes the key and returns `true` if it was there, or `false` if it was not. It changes `m`.

```javascript
let m: map<string, int> = ["a": 1]
print(@map.remove(m, "a"), @map.remove(m, "a"))   // true false
```

### Listing

**`@map.keys(m)`** returns an array with all the keys, in insertion order.

**`@map.values(m)`** returns an array with all the values, in the same order as the keys.

```javascript
let ages: map<string, int> = ["ana": 31, "bia": 26]

print(@map.keys(ages))     // [ana, bia]
print(@map.values(ages))   // [31, 26]
```

**`@map.keysOf(m, v)`** returns an array with every key whose value is `v`. If several keys share the value, all of them come back. If none do, the result is empty. The type of `v` must be the value type of the map.

```javascript
let ages: map<string, int> = ["ana": 31, "bia": 26]

print(@map.keysOf(ages, 31))   // [ana]
print(@map.keysOf(ages, 99))   // []
```

### Transforming maps

**`@map.filter(m, fn(k: K, v: V) -> bool)`** returns a new map with the entries for which the function returns `true`.

**`@map.map(m, fn(k: K, v: V) -> U)`** returns a new `map<K, U>`, with the same keys and the result of the function as each value.

Both work in insertion order:

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

### Errors

Wrong key type:

```javascript
let ages: map<string, int> = ["ana": 30]
print(@map.has(ages, 1))
// '@map.has' argument #2 expects string, got int
```

Wrong fallback type:

```javascript
print(@map.get(ages, "ana", "x"))
// '@map.get' argument #3 expects int, got string
```

Map literal with mixed types:

```javascript
let m = ["a": 1, "b": "x"]
// this map's values are int, found string
```

## Limits

- Write the type on an empty `[]` and `[:]`, as described in [collections.md](../about/collections.md).
- `@map.remove` is slower than the other map functions on very large maps, because it reorders the entries after the removed one.
