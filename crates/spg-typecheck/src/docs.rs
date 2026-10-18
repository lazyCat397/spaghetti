//! one-line descriptions for the built-ins, shown by the editor next to each name.
//! plain english, straight to the point. the signatures themselves live in `builtins`.

/// what a built-in does. `name` is the full name: `@math.abs`, `print`, `@ps.run`...
pub fn builtin_doc(name: &str) -> Option<&'static str> {
    Some(match name {
        // ---- globals ----
        "print" => "print a value and a newline to the output",
        "printError" => "print a value and a newline to the error output",
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
        "@math.random" => "a random float from 0 up to (not including) 1",
        "@math.pi" => "pi. with a number of decimals, rounds it",
        "@math.e" => "Euler's number. with a number of decimals, rounds it",
        "@math.tau" => "tau (2 * pi). with a number of decimals, rounds it",

        // ---- @into: conversions ----
        "@into.string" => "the value as text",
        "@into.int" => "the number as an int, or the text parsed as an int (error if it isn't one)",
        "@into.float" => "the number as a float, or the text parsed as a float (error if it isn't one)",

        // ---- @time ----
        "@time.now" => "the current date and time as text",
        "@time.unix" => "seconds since 1970, as an int",
        "@time.elapsed" => "time since the program started, as text",
        "@time.sleep" => "pause for n milliseconds. other tasks keep running meanwhile",

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
        "@file.read" => "the whole text of a file",
        "@file.readBytes" => "the bytes of a file, as an array of ints",
        "@file.write" => "replace the file's content with the text",
        "@file.writeBytes" => "replace the file's content with the bytes",
        "@file.readLines" => "the lines of a file, as an array of strings",
        "@file.writeLines" => "replace the file's content with the lines, each ended with a newline",
        "@file.appendLine" => "add the line (plus a newline) at the end of the file",
        "@file.readOr" => "the whole text of a file, or the fallback if the file doesn't exist",
        "@file.append" => "add the text at the end of the file",
        "@file.lines" => "the lines of a file, as an array of strings",
        "@file.exists" => "true if a file or folder is there",
        "@file.isDir" => "true if the path is a folder",
        "@file.list" => "the names inside a folder",
        "@file.mkdir" => "create a folder (and the ones above it)",
        "@file.delete" => "delete a file",
        "@file.deleteAll" => "delete a folder and everything inside it",
        "@file.copy" => "copy a file to another path",
        "@file.rename" => "move or rename a file or folder",
        "@file.join" => "join path parts with the right separator",
        "@file.size" => "size of a file in bytes",
        "@file.modified" => "last modification time, as seconds since 1970",
        "@file.modifiedDate" => "last modification time, as text",

        // ---- @json ----
        "@json.valid" => "true if the text is valid JSON",
        "@json.get" => "the value at a path inside a JSON text",
        "@json.has" => "true if the path exists in a JSON text",
        "@json.type" => "the JSON type at a path: object, array, string, number, bool or null",
        "@json.len" => "number of items of the array or object at a path",
        "@json.keys" => "the keys of the object at a path",
        "@json.set" => "a new JSON text with the value written at a path",
        "@json.stringify" => "the value as compact JSON text",
        "@json.pretty" => "the value as indented JSON text",

        // ---- @task: tasks ----
        "@task.all" => "wait for every task in the array, and give back their results in order",
        "@task.wait" => "wait for the task, and give back its result. re-throws its error",

        // ---- @ps: PowerShell ----
        "@ps.run" => "run a PowerShell command. gives back a PsResult",
        "@ps.runFile" => "run a .ps1 script. gives back a PsResult",
        "@http.get" => "GET a url, optional headers map and timeout in seconds (default 30). gives back an HttpResponse (status, headers, body). a 404 is a normal answer; only no answer at all throws",
        "@http.post" => "POST a body to a url, optional headers map and timeout in seconds. gives back an HttpResponse",
        "@ps.version" => "the PowerShell version, as text",
        "@ps.isAvailable" => "true if PowerShell can be found on this machine",

        _ => return None,
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
        "time" => "clock and time, and sleep",
        "file" => "files and folders",
        "sys" => "the system: args, env, input, exit",
        "json" => "read and write JSON",
        "task" => "tasks: wait for background work",
        "ps" => "PowerShell (install <std: \"ps\">)",
        _ => return None,
    })
}
