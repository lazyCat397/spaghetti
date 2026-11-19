# Data and files: `@file` and `@json`

This page covers two namespaces:

- `@file` reads and writes files and folders on disk.
- `@json` reads, changes and writes JSON text.

Paths are plain strings. Forward slashes (`/`) work on every system, including Windows. The examples below run in the current folder, so they change real files. Try them in an empty folder.

## Files

Each `@file` call reads or writes a whole file at once. There are no streams and no way to read part of a file. A problem with the disk, such as a missing file or no permission, is a runtime error. You can catch it with `try` and `catch`, as shown in the [errors](#errors) section.

### Reading and writing text

**`@file.write(path, text)`** creates the file, or replaces its content if it already exists.

**`@file.append(path, text)`** adds `text` to the end of the file. It creates the file if it does not exist.

**`@file.read(path)`** returns the whole content as a `string`.

```javascript
@file.write("notes.txt", "first line\nsecond line\n")
@file.append("notes.txt", "third line\n")

print(@file.read("notes.txt"))
// first line
// second line
// third line
```

**`@file.readLines(path)`** returns the content as an `array<string>`, one item per line. It accepts both `\n` and `\r\n` line endings, and it leaves out the empty line that comes after the last line break. It is the reading half of `writeLines`.

```javascript
print(@file.readLines("notes.txt"))   // [first line, second line, third line]
```

`read` only works on text. A file with bytes that are not valid UTF-8 is an error. Use `readBytes` for those files, as shown below.

### Checking what exists

**`@file.exists(path)`** returns `true` if the path exists, as a file or as a folder.

**`@file.isDir(path)`** returns `true` if the path is a folder.

```javascript
print(@file.exists("notes.txt"))   // true
print(@file.exists("nope.txt"))    // false
print(@file.isDir("."))            // true
print(@file.isDir("notes.txt"))    // false
```

**`@file.list(folder)`** returns the names of the items in a folder, sorted. The names do not include the folder path.

```javascript
@file.mkdir("data")
@file.write("data/b.txt", "B")
@file.write("data/a.txt", "A")

print(@file.list("data"))   // [a.txt, b.txt]
```

**`@file.join(a, b)`** joins two parts of a path with the separator of the current system:

```javascript
print(@file.join("data", "a.txt"))
// data/a.txt on Linux and macOS, data\a.txt on Windows
```

### Size and dates

**`@file.size(path)`** returns the size in bytes, as an `int`. A folder is an error.

**`@file.modified(path)`** returns the last change as milliseconds since 1970. This is the same unit that `@time.unix()` uses.

**`@file.modifiedDate(path)`** returns the last change as text in UTC, in the same format as `@time.now()`, for example `2026-10-08T22:27:52Z`.

```javascript
print(@file.size("notes.txt"))             // 30
print(@file.modified("notes.txt") > 0)     // true
```

Some systems do not record when a file was created, so Spaghetti does not offer a creation date.

### Folders

**`@file.mkdir(path)`** creates the folder and any missing parent folders. It does nothing if the folder already exists.

```javascript
@file.mkdir("data/deep/inside")
print(@file.isDir("data/deep/inside"))   // true
```

**`@file.copy(from, to)`** copies a file. It does not copy folders.

**`@file.rename(from, to)`** moves or renames a file or folder.

```javascript
@file.copy("data/a.txt", "data/c.txt")
@file.rename("data/c.txt", "data/d.txt")

print(@file.list("data"))   // [a.txt, b.txt, d.txt, deep]
```

### Deleting

Deleting comes in two forms, so you choose when a whole folder can disappear.

**`@file.delete(path)`** removes a file, or an empty folder. It never removes a folder that still has items inside.

**`@file.deleteAll(path)`** removes a file, or a folder with everything inside it. Use it with care. It refuses to delete the root of the file system.

```javascript
@file.delete("data/a.txt")
@file.delete("data/deep/inside")   // empty folder, removed
@file.deleteAll("data")            // the whole folder, with everything in it
```

### Binary files

**`@file.readBytes(path)`** returns the content as an `array<int>`. Each item is a byte, from `0` to `255`.

**`@file.writeBytes(path, bytes)`** writes an `array<int>` as bytes. Every item must be from `0` to `255`. The check happens before anything is written, so a bad item leaves the disk untouched.

```javascript
@file.writeBytes("bytes.bin", [72, 105, 0, 255])
print(@file.readBytes("bytes.bin"))   // [72, 105, 0, 255]
```

### Errors

Errors in `@file` are runtime errors. They stop the program unless you catch them. The message says the function, the path and the reason:

```javascript
print(@file.read("missing.txt"))
// runtime error: @file.read("missing.txt"): no such file or directory
```

Catching a missing file:

```javascript
try {
    print(@file.read("missing.txt"))
} catch (e) {
    print("caught: " + e)
}
// caught: @file.read("missing.txt"): no such file or directory
```

Deleting a folder that still has items:

```javascript
@file.delete("full")
// runtime error: @file.delete("full"): Directory not empty (os error 39)
```

Asking for the size of a folder:

```javascript
print(@file.size("data"))
// runtime error: @file.size("data"): is a directory
```

Writing into a folder that does not exist:

```javascript
@file.write("nodir/none/x.txt", "hi")
// runtime error: @file.write("nodir/none/x.txt"): no such file or directory
```

Reading a file that is not text:

```javascript
@file.writeBytes("hi.bin", [255, 254])
print(@file.read("hi.bin"))
// runtime error: @file.read("hi.bin"): not valid UTF-8 text
```

Writing a byte outside `0` to `255`, which creates nothing:

```javascript
@file.writeBytes("bad.bin", [1, 300])
// runtime error: @file.writeBytes: the item at index 1 is 300; bytes go from 0 to 255
```

## JSON

`@json` works on JSON text, which is a `string`. It does not convert the text into a value. Instead, it reads the text along a path, such as `owner.id`, or it creates new text.

Reading gives back a `string`, an `int`, a `float`, a `bool`, or a `string` for other values. Spaghetti is typed, so arrays and objects come back as their JSON text, which you can read again with the same functions.

### Paths

A path is a list of names separated by dots. A number in the path picks an item from an array. An empty path `""` means the whole document.

```javascript
let text = "{\"owner\": {\"id\": 7}, \"tags\": [\"a\", \"b\"]}"

print(@json.get(text, "owner.id"))   // 7
print(@json.get(text, "tags.1"))     // b
print(@json.get(text, ""))           // {"owner":{"id":7},"tags":["a","b"]}
```

If a key or name in the path has a dot in it, escape the dot with a backslash: `\.`. Use `\\` for a backslash.

### Reading

**`@json.get(text, path)`** returns the value at the path. Strings are returned decoded, without the quotes. Numbers and booleans are returned as text, the way they were written. `null` is returned as `"null"`. Objects and arrays are returned as compact JSON, so you can read them again:

```javascript
let text = "{\"name\": \"spg\", \"size\": 42, \"ok\": true, \"tags\": [\"a\", \"b\"]}"

print(@json.get(text, "name"))   // spg
print(@json.get(text, "size"))   // 42
print(@json.get(text, "ok"))     // true
print(@json.get(text, "tags"))   // ["a","b"]
```

**`@json.getString(text, path, fallback)`**, **`@json.getInt(text, path, fallback)`**, **`@json.getFloat(text, path, fallback)`** and **`@json.getBool(text, path, fallback)`** return the value at the path as the kind you ask for. They are the typed version of `get`, and they work the same way as the `toml` and `json5` libraries. A missing path, or `null`, gives back the fallback. A value of another kind is an error, so a wrong type is never hidden by the fallback. `getInt` only accepts whole numbers, and `getFloat` accepts whole numbers too.

```javascript
let text = "{\"port\": 8080, \"ratio\": 1.50, \"debug\": true}"

print(@json.getInt(text, "port", 3000))      // 8080
print(@json.getFloat(text, "ratio", 1.0))    // 1.5
print(@json.getBool(text, "debug", false))   // true
print(@json.getInt(text, "host", 3000))      // 3000 (missing: the fallback)
print(@json.getInt(text, "ratio", 0))        // error: the value at 'ratio' is a number, not an integer
```

**`@json.has(text, path)`** returns `true` if the path exists. Use it before `get` when the key might be missing, because a missing path is an error for `get`:

```javascript
print(@json.has(text, "name"))   // true
print(@json.has(text, "zz"))     // false
```

**`@json.type(text, path)`** returns the kind of value at the path. The names are `string`, `number`, `bool`, `null`, `array` and `object`:

```javascript
print(@json.type(text, "size"))   // number
print(@json.type(text, "tags"))   // array
```

**`@json.len(text, path)`** returns the number of items in an array or the number of keys in an object. Any other value is an error.

**`@json.keys(text, path)`** returns the keys of an object as an `array<string>`, in the order they appear in the text:

```javascript
print(@json.keys(text, ""))   // [name, size, ok, tags]
```

**`@json.valid(text)`** returns `true` if the text is valid JSON:

```javascript
print(@json.valid("[1, 2]"))   // true
print(@json.valid("[1,]"))     // false
```

### Writing

**`@json.set(text, path, value)`** returns new JSON text with the value at the path. Only the last step of the path can be new: the parent objects and arrays must already exist.

```javascript
let text = "{\"name\": \"spg\", \"owner\": {\"id\": 7}}"

print(@json.set(text, "name", "new"))
// {"name":"new","owner":{"id":7}}

print(@json.set(text, "owner.role", "admin"))
// {"name":"spg","owner":{"id":7,"role":"admin"}}
```

Setting an index equal to the length of an array adds an item to the end:

```javascript
print(@json.set("[1, 2]", "2", 9))   // [1,2,9]
```

`set` keeps the layout of the original text when it can. A text written on one line stays on one line. A text written over several lines keeps the same indentation. Objects or arrays that were on one line become several lines when you add to them.

**`@json.stringify(value)`** turns any Spaghetti value into compact JSON text. Structs become objects with their field names. Enums become the name of the variant. Map keys become text:

```javascript
struct Cfg {
    host: string,
    port: int,
    debug: bool
}

let c = Cfg { host: "localhost", port: 8080, debug: false }
print(@json.stringify(c))
// {"host":"localhost","port":8080,"debug":false}

enum Mode { Fast, Slow }
print(@json.stringify(Mode.Slow))   // "Slow"

let m: map<string, int> = ["a": 1]
print(@json.stringify(m))           // {"a":1}
```

Floats keep their decimal point: `@json.stringify(1.0)` gives `1.0`.

**`@json.pretty(value)`** works like `stringify`, but puts each item on its own line with indentation:

```javascript
print(@json.pretty([[1, 2], [3]]))
// [
//   [
//     1,
//     2
//   ],
//   [
//     3
//   ]
// ]
```

`pretty` takes a value. If you give it a text, it returns that text as a JSON string, with quotes and escapes. It does not reformat the JSON inside the text:

```javascript
print(@json.pretty("{\"a\": 1}"))   // "{\"a\": 1}"
```

Values that have no JSON form are errors. These are `NaN`, infinity, and `void`:

```javascript
print(@json.stringify(0.0 / 0.0))
// runtime error: @json.stringify: NaN has no JSON form (JSON has no NaN or infinity)
```

### Errors

Errors in `@json` are runtime errors, and you can catch them with `try`. A missing path is an error for `get`, and the message says where the search stopped:

```javascript
print(@json.get("{\"a\": 1}", "b"))
// runtime error: @json.get: path not found: 'b' (the root object has no key 'b')
```

Invalid JSON gives the line and column:

```javascript
print(@json.get("{oops", "a"))
// runtime error: @json.get: invalid JSON: expected a string key (line 1, column 2)
```

`len` only counts arrays and objects:

```javascript
print(@json.len("5", ""))
// runtime error: @json.len: '' is a number: len only measures arrays and objects
```

`set` can only create the last step of a path:

```javascript
print(@json.set("{\"a\": 1}", "b.c", 1))
// runtime error: @json.set: path not found: 'b' (set only creates the LAST step of a path; the object has no key 'b')
```

Index out of range in `set`:

```javascript
print(@json.set("[1, 2]", "5", 9))
// runtime error: @json.set: path not found: '5' (index 5 is out of range: the root array has 2 item(s))
```

## Limits

- `@file` works on whole files only. There are no streams, no partial reads, and no wildcards.
- `@file.read` only works on text. Use `readBytes` for other files.
- `@file.copy` copies one file. It does not copy folders.
- `@json` returns text for values. It does not turn JSON into a struct. Read each field with `get`.
- `@json.set` cannot create more than the last step of a path.
