//! one-line descriptions for the built-ins, shown by the editor next to each name.
//! plain english, straight to the point. the signatures themselves live in `builtins`.

/// what a built-in does. `name` is the full name: `@math.abs`, `print`, `@ps.run`...
pub fn builtin_doc(name: &str) -> Option<&'static str> {
    Some(match name {
        // ---- globals ----
        "print" => "print a value and a newline to the output",
        "printError" => "print a value and a newline to the error output",
        "type" => "the type of a value, as text: int, float, bool, string, array, map, function, task, none, or the declared name of a struct or enum",
        "len" => "number of characters of a string, or elements of an array",
        "readLine" => "read one line from the input, with an optional prompt",

        // ---- @str: text ----
        "@str.len" => "number of characters",
        "@str.toUpper" => "the text in upper case",
        "@str.toLower" => "the text in lower case",
        "@str.trim" => "the text without spaces at both ends",
        "@str.trimStart" => "the text without spaces at the start",
        "@str.trimEnd" => "the text without spaces at the end",
        "@str.split" => "split the text by a separator, into an array of strings",
        "@str.substring" => "characters from start up to (not including) end",
        "@str.slice" => "characters from start up to (not including) end",
        "@str.charAt" => "the character at an index",
        "@str.contains" => "true if the text has the part you pass",
        "@str.startsWith" => "true if the text starts with the prefix",
        "@str.endsWith" => "true if the text ends with the suffix",
        "@str.replace" => "replace every occurrence of a part with another text",
        "@str.indexOf" => "index of the first occurrence, or -1",
        "@str.lastIndexOf" => "index of the last occurrence, or -1",
        "@str.repeat" => "the text repeated n times",
        "@str.padStart" => "pad the start up to a width (space by default)",
        "@str.padEnd" => "pad the end up to a width (space by default)",

        // ---- @array: lists ----
        "@array.len" => "number of elements",
        "@array.push" => "add an element at the end",
        "@map.map" => "a new map with f applied to each entry: @map.map(m, fn(k: K, v: V) -> U) gives map<K, U>",
        "@map.filter" => "a new map with the entries where f gives true: @map.filter(m, fn(k: K, v: V) -> bool)",
        "@array.first" => "the first element, or none for an empty array: @array.first(arr) gives T?",
        "@array.last" => "the last element, or none for an empty array: @array.last(arr) gives T?",
        "@array.map" => "a new array with f applied to each element: @array.map(arr, fn(x: T) -> U) gives U[]",
        "@array.filter" => "a new array with the elements where f gives true: @array.filter(arr, fn(x: T) -> bool)",
        "@array.reduce" => "folds the array into one value: @array.reduce(arr, fn(acc: A, x: T) -> A, init)",
        "@array.pop" => "remove and return the last element",
        "@array.insert" => "insert an element at an index",
        "@array.remove" => "remove and return the element at an index",
        "@array.clear" => "remove every element",
        "@array.contains" => "true if the array has the element",
        "@array.indexOf" => "index of the first matching element, or -1",
        "@array.slice" => "a new array from start up to (not including) end",
        "@array.concat" => "a new array with both arrays' elements, one after the other",
        "@array.reverse" => "the array in reverse order",
        "@array.sort" => "the array sorted, smallest first",
        "@array.join" => "join the strings with a separator",
        "@array.sum" => "the sum of the numbers",
        "@array.range" => "integers from start up to (not including) end, with an optional step",
        "@array.fill" => "an array of n copies of a value",

        // ---- @map: key/value pairs ----
        "@map.len" => "number of pairs",
        "@map.has" => "true if the key exists",
        "@map.get" => "the value for a key, or the fallback if it's missing",
        "@map.remove" => "remove a key, true if it was there",
        "@map.set" => "set the value for a key: adds the key, or replaces its value. same as m[key] = value",
        "@map.keys" => "every key, as an array",
        "@map.values" => "every value, as an array",
        "@map.keysOf" => "every key that maps to the value",

        // ---- @math: numbers ----
        "@math.abs" => "absolute value. int stays int, float stays float",
        "@math.sign" => "-1, 0 or 1 for the sign of a number",
        "@math.min" => "the smaller of two numbers",
        "@math.max" => "the bigger of two numbers",
        "@math.clamp" => "keep a number between a low and a high bound",
        "@math.floor" => "round down to the nearest whole number",
        "@math.ceil" => "round up to the nearest whole number",
        "@math.round" => "round to the nearest whole number",
        "@math.sqrt" => "square root",
        "@math.pow" => "base raised to the exponent",
        "@math.exp" => "e raised to the power of x",
        "@math.log" => "natural logarithm",
        "@math.sin" => "sine, the angle in radians",
        "@math.cos" => "cosine, the angle in radians",
        "@math.tan" => "tangent, the angle in radians",
        "@math.asin" => "arc sine, in radians",
        "@math.acos" => "arc cosine, in radians",
        "@math.atan" => "arc tangent, in radians",
        "@math.atan2" => "arc tangent of y / x, in radians, with the right quadrant",
        "@math.toDegrees" => "convert radians to degrees",
        "@math.toRadians" => "convert degrees to radians",
        "@math.random" => "a random number. no arguments: a float from 0 up to (not including) 1. (lo, hi) ints: an int from lo to hi, both included. (lo, hi) floats: a float from lo up to hi",
        "@math.seed" => "fix the random sequence from this seed, so the same seed gives the same numbers",
        "@array.pick" => "a random element of the array (error if it is empty)",
        "@array.shuffle" => "a new array with the same elements in a random order",
        "@math.pi" => "pi. with a number of decimals, rounds it",
        "@math.e" => "Euler's number. with a number of decimals, rounds it",
        "@math.tau" => "tau (2 * pi). with a number of decimals, rounds it",

        // ---- @into: conversions ----
        "@into.string" => "the value as text",
        "@into.int" => "the number as an int, or the text parsed as an int (error if it isn't one)",
        "@into.float" => "the number as a float, or the text parsed as a float (error if it isn't one)",

        // ---- @time ----

        // ---- @sys: the system ----
        "@sys.args" => "the command-line arguments, as an array of strings",
        "@sys.env" => "the value of an environment variable, or the fallback",
        "@sys.cwd" => "the current working folder",
        "@sys.platform" => "the operating system name",
        "@sys.exit" => "end the program now with an exit code",
        "@sys.clear" => "clear the terminal screen and its scrollback",
        "@sys.readLine" => "read one line from the input",
        "@sys.readAll" => "read everything left from the input",

        // ---- @file ----

        // ---- @json ----

        // ---- @task: tasks ----
        "@task.all" => "wait for every task in the array, and give back their results in order",
        "@task.wait" => "wait for the task, and give back its result. re-throws its error",

        // the native libraries (ps, http, ...) document themselves: see `lib_doc`
        _ => return lib_doc(name),
    })
}

/// what a namespace is for, shown when picking it after `@`
pub fn namespace_doc(ns: &str) -> Option<&'static str> {
    Some(match ns {
        "str" => "text: searching, splitting, padding",
        "array" => "lists: push, pop, sort, slices",
        "map" => "key/value pairs",
        "math" => "numbers: rounding, roots, trigonometry, random",
        "into" => "conversions between int, float and string",
        "sys" => "the system: args, env, input, exit",
        "task" => "tasks: wait for background work",
        _ => return spg_libs::module(ns).map(|m| m.doc),
    })
}

/// the docs of a native library's function, from its own descriptor (`"@ps.run"` -> ps / run)
fn lib_doc(name: &str) -> Option<&'static str> {
    let (module, function) = name.strip_prefix('@')?.split_once('.')?;
    spg_libs::module(module)?.functions.iter().find(|f| f.name == function).map(|f| f.doc)
}
