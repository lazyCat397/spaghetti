//! `@file`: files and folders. part of the core (always there, written with `@`).

use std::io::Write;
use std::time::UNIX_EPOCH;

use crate::util::iso_utc;
use crate::{CallCtx, Kind, LibError, LibValue, NativeFn, NativeModule};

/// the most bytes `@file.readBytes` hands back (an array of that many ints is already a lot)
const MAX_BYTES: usize = 20_000_000;

pub static MODULE: NativeModule = NativeModule {
    kind: Kind::Core,
    name: "file",
    doc: "files and folders",
    structs: &[],
    functions: &[
        NativeFn { name: "read", sig: "(string) -> string", doc: "the whole text of a file", call: read },
        NativeFn { name: "write", sig: "(string, string) -> void", doc: "replace the file's content with the text", call: write },
        NativeFn { name: "append", sig: "(string, string) -> void", doc: "add the text at the end of the file", call: append },
        NativeFn { name: "lines", sig: "(string) -> array<string>", doc: "the lines of a file, as an array of strings", call: lines },
        NativeFn { name: "readLines", sig: "(string) -> array<string>", doc: "the lines of a file, as an array of strings", call: read_lines },
        NativeFn { name: "writeLines", sig: "(string, array<string>) -> void", doc: "replace the file's content with the lines, each ended with a newline", call: write_lines },
        NativeFn { name: "appendLine", sig: "(string, string) -> void", doc: "add the line (plus a newline) at the end of the file", call: append_line },
        NativeFn { name: "readOr", sig: "(string, string) -> string", doc: "the whole text of a file, or the fallback if the file doesn't exist", call: read_or },
        NativeFn { name: "exists", sig: "(string) -> bool", doc: "true if a file or folder is there", call: exists },
        NativeFn { name: "isDir", sig: "(string) -> bool", doc: "true if the path is a folder", call: is_dir },
        NativeFn { name: "list", sig: "(string) -> array<string>", doc: "the names inside a folder", call: list },
        NativeFn { name: "mkdir", sig: "(string) -> void", doc: "create a folder (and the ones above it)", call: mkdir },
        NativeFn { name: "delete", sig: "(string) -> void", doc: "delete a file", call: delete },
        NativeFn { name: "copy", sig: "(string, string) -> void", doc: "copy a file to another path", call: copy },
        NativeFn { name: "rename", sig: "(string, string) -> void", doc: "move or rename a file or folder", call: rename },
        NativeFn { name: "join", sig: "(string, string) -> string", doc: "join path parts with the right separator", call: join },
        NativeFn { name: "deleteAll", sig: "(string) -> void", doc: "delete a folder and everything inside it", call: delete_all },
        NativeFn { name: "size", sig: "(string) -> int", doc: "size of a file in bytes", call: size },
        NativeFn { name: "modified", sig: "(string) -> int", doc: "last modification time, as milliseconds since 1970", call: modified },
        NativeFn { name: "modifiedDate", sig: "(string) -> string", doc: "last modification time, as text", call: modified_date },
        NativeFn { name: "readBytes", sig: "(string) -> array<int>", doc: "the bytes of a file, as an array of ints", call: read_bytes },
        NativeFn { name: "writeBytes", sig: "(string, array<int>) -> void", doc: "replace the file's content with the bytes", call: write_bytes },
    ],
};

type Out = Result<LibValue, LibError>;

fn io(function: &'static str, path: &str, e: std::io::Error) -> LibError {
    use std::io::ErrorKind;
    let reason = match e.kind() {
        ErrorKind::NotFound => "no such file or directory".to_string(),
        ErrorKind::PermissionDenied => "permission denied".to_string(),
        ErrorKind::AlreadyExists => "already exists".to_string(),
        ErrorKind::InvalidData => "not valid UTF-8 text".to_string(),
        _ => e.to_string(),
    };
    LibError::io(function, path, reason)
}

/// argument `i` as text (the typechecker already guaranteed it is a string)
fn s(args: &[LibValue], i: usize) -> &str {
    match args.get(i) {
        Some(LibValue::Str(text)) => text.as_str(),
        _ => "",
    }
}

fn str_array(items: impl Iterator<Item = String>) -> LibValue {
    LibValue::Array(items.map(LibValue::Str).collect())
}

fn read(_: &CallCtx, a: Vec<LibValue>) -> Out {
    let p = s(&a, 0);
    std::fs::read_to_string(p).map(LibValue::Str).map_err(|e| io("@file.read", p, e))
}

fn write(_: &CallCtx, a: Vec<LibValue>) -> Out {
    let p = s(&a, 0);
    std::fs::write(p, s(&a, 1).as_bytes()).map_err(|e| io("@file.write", p, e))?;
    Ok(LibValue::Void)
}

fn append(_: &CallCtx, a: Vec<LibValue>) -> Out {
    let p = s(&a, 0);
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(p).map_err(|e| io("@file.append", p, e))?;
    file.write_all(s(&a, 1).as_bytes()).map_err(|e| io("@file.append", p, e))?;
    Ok(LibValue::Void)
}

fn lines_of(function: &'static str, p: &str) -> Out {
    let text = std::fs::read_to_string(p).map_err(|e| io(function, p, e))?;
    Ok(str_array(text.lines().map(String::from)))
}

fn lines(_: &CallCtx, a: Vec<LibValue>) -> Out {
    lines_of("@file.lines", s(&a, 0))
}

fn read_lines(_: &CallCtx, a: Vec<LibValue>) -> Out {
    lines_of("@file.readLines", s(&a, 0))
}

fn write_lines(_: &CallCtx, a: Vec<LibValue>) -> Out {
    let p = s(&a, 0);
    let mut text = String::new();
    if let Some(LibValue::Array(items)) = a.get(1) {
        for item in items {
            if let LibValue::Str(line) = item {
                text.push_str(line);
                text.push('\n');
            }
        }
    }
    std::fs::write(p, text.as_bytes()).map_err(|e| io("@file.writeLines", p, e))?;
    Ok(LibValue::Void)
}

fn append_line(_: &CallCtx, a: Vec<LibValue>) -> Out {
    let p = s(&a, 0);
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(p).map_err(|e| io("@file.appendLine", p, e))?;
    file.write_all(s(&a, 1).as_bytes()).map_err(|e| io("@file.appendLine", p, e))?;
    file.write_all(b"\n").map_err(|e| io("@file.appendLine", p, e))?;
    Ok(LibValue::Void)
}

fn read_or(_: &CallCtx, a: Vec<LibValue>) -> Out {
    let p = s(&a, 0);
    // a missing file is the whole point of the fallback; any other failure still errors
    match std::fs::read_to_string(p) {
        Ok(text) => Ok(LibValue::Str(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(LibValue::Str(s(&a, 1).to_string())),
        Err(e) => Err(io("@file.readOr", p, e)),
    }
}

fn exists(_: &CallCtx, a: Vec<LibValue>) -> Out {
    Ok(LibValue::Bool(std::path::Path::new(s(&a, 0)).exists()))
}

fn is_dir(_: &CallCtx, a: Vec<LibValue>) -> Out {
    Ok(LibValue::Bool(std::path::Path::new(s(&a, 0)).is_dir()))
}

fn list(_: &CallCtx, a: Vec<LibValue>) -> Out {
    let p = s(&a, 0);
    let mut names: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(p).map_err(|e| io("@file.list", p, e))? {
        let entry = entry.map_err(|e| io("@file.list", p, e))?;
        names.push(entry.file_name().to_string_lossy().into_owned());
    }
    names.sort();
    Ok(str_array(names.into_iter()))
}

fn mkdir(_: &CallCtx, a: Vec<LibValue>) -> Out {
    let p = s(&a, 0);
    std::fs::create_dir_all(p).map_err(|e| io("@file.mkdir", p, e))?;
    Ok(LibValue::Void)
}

/// a file, or an EMPTY directory. never recursive. no thanks.
fn delete(_: &CallCtx, a: Vec<LibValue>) -> Out {
    let p = s(&a, 0);
    let path = std::path::Path::new(p);
    let done = if path.is_dir() { std::fs::remove_dir(path) } else { std::fs::remove_file(path) };
    done.map_err(|e| io("@file.delete", p, e))?;
    Ok(LibValue::Void)
}

fn copy(_: &CallCtx, a: Vec<LibValue>) -> Out {
    let from = s(&a, 0);
    std::fs::copy(from, s(&a, 1)).map_err(|e| io("@file.copy", from, e))?;
    Ok(LibValue::Void)
}

fn rename(_: &CallCtx, a: Vec<LibValue>) -> Out {
    let from = s(&a, 0);
    std::fs::rename(from, s(&a, 1)).map_err(|e| io("@file.rename", from, e))?;
    Ok(LibValue::Void)
}

fn join(_: &CallCtx, a: Vec<LibValue>) -> Out {
    Ok(LibValue::Str(std::path::Path::new(s(&a, 0)).join(s(&a, 1)).to_string_lossy().into_owned()))
}

/// a file or a whole directory tree. refuses a filesystem root. we're not monsters.
fn delete_all(_: &CallCtx, a: Vec<LibValue>) -> Out {
    let p = s(&a, 0);
    let path = std::path::Path::new(p);
    if let Ok(real) = std::fs::canonicalize(path) {
        if real.parent().is_none() {
            return Err(LibError::io("@file.deleteAll", p, "refusing to delete a filesystem root"));
        }
    }
    let done = if path.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) };
    done.map_err(|e| io("@file.deleteAll", p, e))?;
    Ok(LibValue::Void)
}

fn size(_: &CallCtx, a: Vec<LibValue>) -> Out {
    let p = s(&a, 0);
    let meta = std::fs::metadata(p).map_err(|e| io("@file.size", p, e))?;
    if meta.is_dir() {
        return Err(LibError::io("@file.size", p, "is a directory"));
    }
    Ok(LibValue::Int(meta.len() as i64))
}

/// milliseconds since 1970 of the last modification
fn modified_millis(function: &'static str, p: &str) -> Result<i64, LibError> {
    let when = std::fs::metadata(p).and_then(|m| m.modified()).map_err(|e| io(function, p, e))?;
    Ok(match when.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_millis() as i64,
        Err(e) => -(e.duration().as_millis() as i64),
    })
}

fn modified(_: &CallCtx, a: Vec<LibValue>) -> Out {
    modified_millis("@file.modified", s(&a, 0)).map(LibValue::Int)
}

fn modified_date(_: &CallCtx, a: Vec<LibValue>) -> Out {
    let millis = modified_millis("@file.modifiedDate", s(&a, 0))?;
    Ok(LibValue::Str(iso_utc(millis.div_euclid(1000))))
}

fn read_bytes(_: &CallCtx, a: Vec<LibValue>) -> Out {
    let p = s(&a, 0);
    let bytes = std::fs::read(p).map_err(|e| io("@file.readBytes", p, e))?;
    if bytes.len() > MAX_BYTES {
        return Err(LibError::io("@file.readBytes", p, format!("the file has {} bytes; the most it can read is {}", bytes.len(), MAX_BYTES)));
    }
    Ok(LibValue::Array(bytes.into_iter().map(|b| LibValue::Int(i64::from(b))).collect()))
}

fn write_bytes(_: &CallCtx, a: Vec<LibValue>) -> Out {
    let p = s(&a, 0);
    let mut bytes: Vec<u8> = Vec::new();
    if let Some(LibValue::Array(items)) = a.get(1) {
        for (index, item) in items.iter().enumerate() {
            if let LibValue::Int(value) = item {
                // checked BEFORE touching the disk. a bad item must not leave a half-written file
                match u8::try_from(*value) {
                    Ok(b) => bytes.push(b),
                    Err(_) => {
                        return Err(LibError::new(
                            "@file.writeBytes",
                            format!("the item at index {} is {}; bytes go from 0 to 255", index, value),
                        ))
                    }
                }
            }
        }
    }
    std::fs::write(p, &bytes).map_err(|e| io("@file.writeBytes", p, e))?;
    Ok(LibValue::Void)
}
