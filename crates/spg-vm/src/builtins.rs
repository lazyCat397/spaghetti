//! Runtime side of every builtin. `Vm::step` pops `argc` values
//! for a `CallBuiltin { id, .. }` and gives them to `call_builtin`, which
//! is one giant match over every `BuiltinId`. the counterpart to
//! `spg_typecheck::builtins::builtin_signatures` on the compile-time side.
//! both keyed off the same `BuiltinId` enum, so adding a builtin is two
//! table edits and zero new opcodes. the way it should be.

use std::io::{BufWriter, Write};
use std::rc::Rc;
use std::sync::{Mutex, MutexGuard, Once, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use spg_typecheck::BuiltinId;

use crate::error::RuntimeError;
use crate::value::Value;

// ---- buffered stdout (because println is a crime) ----
//
// `println!` does one write syscall per line. 1M prints: ~330 ms to /dev/null,
// ~1 s to a pipe, ~1.2 us per line in a terminal. so `print` writes
// into one shared 64 KB `BufWriter`. terminals and pipes get the same deal.
//
// output still has to show up when the program goes quiet to compute
// (print "starting...", then crunch for 10 s). so a tiny flusher thread wakes
// every `FLUSH_EVERY` and flushes whatever is pending. nothing waits longer than
// that. the buffer also gets flushed synchronously before
// anything that could mix with it or wait on the outside world
// (`readLine`, `printError`, `@time.sleep`), when `Vm::run` ends (ok or
// error), and from the CLI's panic hook (`flush_stdout_in_panic`). crashes flush too. classy.

const FLUSH_EVERY: Duration = Duration::from_millis(10);

static OUT: Mutex<Option<BufWriter<std::io::Stdout>>> = Mutex::new(None);
static FLUSHER: Once = Once::new();

fn lock_out() -> MutexGuard<'static, Option<BufWriter<std::io::Stdout>>> {
    // a poisoned lock just means some thread panicked while printing. the
    // buffer itself is still fine. probably. don't poke it.
    OUT.lock().unwrap_or_else(|e| e.into_inner())
}

fn start_flusher() {
    // if the thread can't spawn we lose the "prompt while computing"
    // guarantee. output still gets flushed at every synchronous point.
    let _ = std::thread::Builder::new().name("spg-flush".into()).spawn(|| loop {
        std::thread::sleep(FLUSH_EVERY);
        if let Some(w) = lock_out().as_mut() {
            let _ = w.flush();
        }
    });
}

/// `print(a, b, c)` -> `a b c` + newline. values joined by ONE space (like
/// Python). `print()` is just a newline. one arg keeps the original
/// single `writeln!`, so the common case costs exactly what it did before.
fn write_joined<W: Write>(w: &mut W, vs: &[Value]) -> std::io::Result<()> {
    match vs {
        [one] => writeln!(w, "{}", one),
        _ => {
            for (i, v) in vs.iter().enumerate() {
                if i > 0 {
                    w.write_all(b" ")?;
                }
                write!(w, "{}", v)?;
            }
            w.write_all(b"\n")
        }
    }
}

/// writes text to the same buffered stdout as `print`, WITHOUT a newline (for prompts)
fn write_out_text(text: &str) -> Result<(), RuntimeError> {
    let mut guard = lock_out();
    let w = guard.get_or_insert_with(|| BufWriter::with_capacity(64 * 1024, std::io::stdout()));
    w.write_all(text.as_bytes()).map_err(|e| io_error("readLine", "", e))?;
    drop(guard);
    FLUSHER.call_once(start_flusher);
    Ok(())
}

/// one line from stdin, no newline. shared by `readLine` and
/// `@sys.readLine`. flushes stdout first (the prompt has to be visible while we
/// wait). `EndOfInput` when stdin is already done. 
fn read_stdin_line(function: &'static str) -> Result<String, RuntimeError> {
    flush_stdout();
    let mut line = String::new();
    let n = std::io::stdin().read_line(&mut line).map_err(|e| io_error(function, "", e))?;
    if n == 0 {
        return Err(RuntimeError::EndOfInput { function });
    }
    while line.ends_with('\n') || line.ends_with('\r') {
        line.pop();
    }
    Ok(line)
}

fn print_values(vs: &[Value]) {
    let result = {
        let mut guard = lock_out();
        let w = guard.get_or_insert_with(|| BufWriter::with_capacity(64 * 1024, std::io::stdout()));
        write_joined(w, vs)
    };
    FLUSHER.call_once(start_flusher);
    // panic AFTER the lock above is released. same message `println!` uses, so
    // the CLI's "Broken pipe" check keeps working. don't break it.
    if let Err(e) = result {
        panic!("failed printing to stdout: {}", e);
    }
}

/// flushes the buffered stdout (no-op before the first `print`). errors get
/// ignored: a closed pipe here isn't our bug. waits for the lock, so
/// when it returns, everything printed before has reached the OS (even if
/// the flusher thread was mid-flush). good.
/// Windows consoles ignore ANSI escapes until VT processing is switched on. thanks, windows.
/// done once, quietly. a no-op everywhere else. no crate: direct kernel32 calls, sorry.
#[cfg(windows)]
fn enable_ansi_once() {
    use std::ffi::c_void;
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        #[link(name = "kernel32")]
        extern "system" {
            fn GetStdHandle(n: u32) -> *mut c_void;
            fn GetConsoleMode(h: *mut c_void, mode: *mut u32) -> i32;
            fn SetConsoleMode(h: *mut c_void, mode: u32) -> i32;
        }
        const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
        const ENABLE_VIRTUAL_TERMINAL_PROCESSING: u32 = 0x0004;
        unsafe {
            let h = GetStdHandle(STD_OUTPUT_HANDLE);
            let mut mode = 0u32;
            if GetConsoleMode(h, &mut mode) != 0 {
                SetConsoleMode(h, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
            }
        }
    });
}

#[cfg(not(windows))]
fn enable_ansi_once() {}


pub fn flush_stdout() {
    if let Some(w) = lock_out().as_mut() {
        let _ = w.flush();
    }
}

/// same, for the panic hook. the hook runs BEFORE unwinding, so the panicking
/// thread may still hold the lock. never block: if someone has it, they're
/// printing or flushing and the data is on its way. chill.
pub fn flush_stdout_in_panic() {
    if let Ok(mut guard) = OUT.try_lock() {
        if let Some(w) = guard.as_mut() {
            let _ = w.flush();
        }
    }
}

// ---- argument extraction (trust the checker, mostly) ----
// tiny helpers so each arm reads as "the shape I expect". they panic only on a
// real compiler/checker bug (wrong Value variant reaching a builtin that
// the typechecker already constrained). if this fires, blame the checker. or me.
#[inline(always)]
fn as_string(v: Value) -> Rc<String> {
    match v {
        Value::String(s) => s,
        other => panic!("VM bug: builtin expected string, got {}", other.type_name()),
    }
}
#[inline(always)]
fn as_int(v: Value) -> i64 {
    match v {
        Value::Int(n) => n,
        other => panic!("VM bug: builtin expected int, got {}", other.type_name()),
    }
}
#[inline(always)]
fn as_float(v: Value) -> f64 {
    match v {
        Value::Float(n) => n,
        other => panic!("VM bug: builtin expected float, got {}", other.type_name()),
    }
}
#[inline(always)]
fn as_map(v: Value) -> crate::value::MapRef {
    match v {
        Value::Map(m) => m,
        other => panic!("VM bug: builtin expected map, got {}", other.type_name()),
    }
}

fn as_array(v: Value) -> crate::value::ArrayRef {
    match v {
        Value::Array(a) => a,
        other => panic!("VM bug: builtin expected array, got {}", other.type_name()),
    }
}

/// `CallBuiltin { id, argc }` straight on the operand stack. the hot, simple
/// builtins live here, no argument `Vec`, no entering
/// `call_builtin`, whose giant `match` made even `@array.len` cost ~48 ns. ouch.
/// everything else (and every builtin that can fail in a way that needs
/// context) still goes through `call_builtin`. each arm is the
/// same expression as its `call_builtin` twin, operands popped in
/// reverse (last arg first). yes, this is a copy paste. yes, it's fine.
#[inline(never)]
pub(crate) fn call_builtin_on_stack(stack: &mut Vec<Value>, id: BuiltinId, argc: usize) -> Result<(), RuntimeError> {
    use BuiltinId::*;
    macro_rules! pop {
        () => {
            stack.pop().ok_or(RuntimeError::StackUnderflow)?
        };
    }
    let result = match id {
        ArrayLenBuiltin => {
            let a = as_array(pop!());
            let n = a.borrow().len() as i64;
            Value::Int(n)
        }
        Len => Value::Int(as_string(pop!()).chars().count() as i64),
        ArrayContains => {
            let needle = pop!();
            let arr = as_array(pop!());
            let found = arr.borrow().iter().any(|v| v.structurally_equal(&needle));
            Value::Bool(found)
        }
        ArrayPush => {
            let item = pop!();
            let arr = as_array(pop!());
            arr.borrow_mut().push(item);
            Value::Void
        }
        ArrayPop => {
            let arr = as_array(pop!());
            let popped = arr.borrow_mut().pop();
            match popped {
                Some(v) => v,
                None => return Err(RuntimeError::EmptyArray { function: "@array.pop" }),
            }
        }
        Abs => Value::Int(crate::arith::abs(as_int(pop!()))?),
        AbsFloat => Value::Float(as_float(pop!()).abs()),
        MinInt => {
            let b = as_int(pop!());
            let a = as_int(pop!());
            Value::Int(a.min(b))
        }
        MaxInt => {
            let b = as_int(pop!());
            let a = as_int(pop!());
            Value::Int(a.max(b))
        }
        MinFloat => {
            let b = as_float(pop!());
            let a = as_float(pop!());
            Value::Float(a.min(b))
        }
        MaxFloat => {
            let b = as_float(pop!());
            let a = as_float(pop!());
            Value::Float(a.max(b))
        }
        Floor => Value::Int(as_float(pop!()).floor() as i64),
        Ceil => Value::Int(as_float(pop!()).ceil() as i64),
        Round => Value::Int(as_float(pop!()).round() as i64),
        Sqrt => Value::Float(as_float(pop!()).sqrt()),
        IntToFloat => Value::Float(as_int(pop!()) as f64),
        FloatToInt => Value::Int(as_float(pop!()) as i64),
        _ => {
            // generic path: operands popped in reverse push order, same as Call/MakeArray
            let len = stack.len();
            if len < argc {
                return Err(RuntimeError::StackUnderflow);
            }
            let args: Vec<Value> = stack.drain(len - argc..).collect();
            call_builtin(id, args)?
        }
    };
    stack.push(result);
    Ok(())
}

pub fn call_builtin(id: BuiltinId, mut args: Vec<Value>) -> Result<Value, RuntimeError> {
    use BuiltinId::*;

    Ok(match id {
        // ---- I/O (the part that touches the outside world, scary) ----
        Print => {
            print_values(&args);
            Value::Void
        }
        PrintError => {
            flush_stdout(); // keep stdout/stderr order when both go to one place
            let mut line = Vec::new();
            let _ = write_joined(&mut line, &args);
            let _ = std::io::stderr().write_all(&line);
            Value::Void
        }
        ReadLine => Value::String(Rc::new(read_stdin_line("readLine")?)),
        ReadLinePrompt => {
            // the prompt goes out with no newline, so the typed text lands right after it
            write_out_text(&as_string(args.remove(0)))?;
            Value::String(Rc::new(read_stdin_line("readLine")?))
        }

        // ---- strings (utf-8 is a whole mood) ----
        Len => Value::Int(as_string(args.remove(0)).chars().count() as i64),
        ToUpper => Value::String(Rc::new(as_string(args.remove(0)).to_uppercase())),
        ToLower => Value::String(Rc::new(as_string(args.remove(0)).to_lowercase())),
        Trim => Value::String(Rc::new(as_string(args.remove(0)).trim().to_string())),
        Split => {
            let s = as_string(args.remove(0));
            let sep = as_string(args.remove(0));
            let parts: Vec<Value> = if sep.is_empty() {
                s.chars().map(|c| Value::String(Rc::new(c.to_string()))).collect()
            } else {
                s.split(sep.as_str()).map(|p| Value::String(Rc::new(p.to_string()))).collect()
            };
            Value::Array(Rc::new(std::cell::RefCell::new(parts)))
        }
        Substring => {
            let s = as_string(args.remove(0));
            let start = as_int(args.remove(0));
            let end = as_int(args.remove(0));
            // ASCII: char index == byte index. no per-char walk, no Vec<char>. pure joy.
            let ascii = s.is_ascii();
            let len = if ascii { s.len() } else { s.chars().count() };
            if start < 0 || end < start || end > len as i64 {
                return Err(RuntimeError::IndexOutOfBounds { index: end, len });
            }
            let (start, end) = (start as usize, end as usize);
            let slice: String = if ascii {
                s[start..end].to_string()
            } else {
                s.chars().skip(start).take(end - start).collect()
            };
            Value::String(Rc::new(slice))
        }
        CharAt => {
            let s = as_string(args.remove(0));
            let idx = as_int(args.remove(0));
            if idx >= 0 {
                let u = idx as usize;
                // if every byte up to and including `u` is ASCII, char `u` is byte `u`.
                // (the old version built a whole Vec<char> per call. embarrassing.)
                if let Some(prefix) = s.as_bytes().get(..=u) {
                    if prefix.is_ascii() {
                        return Ok(Value::String(Rc::new((prefix[u] as char).to_string())));
                    }
                }
                if let Some(c) = s.chars().nth(u) {
                    return Ok(Value::String(Rc::new(c.to_string())));
                }
            }
            return Err(RuntimeError::IndexOutOfBounds { index: idx, len: s.chars().count() });
        }
        StrContains => {
            let s = as_string(args.remove(0));
            let needle = as_string(args.remove(0));
            Value::Bool(s.contains(needle.as_str()))
        }
        StartsWith => {
            let s = as_string(args.remove(0));
            let prefix = as_string(args.remove(0));
            Value::Bool(s.starts_with(prefix.as_str()))
        }
        EndsWith => {
            let s = as_string(args.remove(0));
            let suffix = as_string(args.remove(0));
            Value::Bool(s.ends_with(suffix.as_str()))
        }
        Replace => {
            let s = as_string(args.remove(0));
            let from = as_string(args.remove(0));
            let to = as_string(args.remove(0));
            Value::String(Rc::new(s.replace(from.as_str(), to.as_str())))
        }
        StrIndexOf => {
            let s = as_string(args.remove(0));
            let needle = as_string(args.remove(0));
            let idx = s
                .char_indices()
                .zip(0i64..)
                .find(|((byte_i, _), _)| s[*byte_i..].starts_with(needle.as_str()))
                .map(|(_, char_i)| char_i)
                .unwrap_or(-1);
            Value::Int(idx)
        }
        // ---- @json (see json.rs, it's a whole thing) ----
        JsonStringify | JsonPretty => {
            let name = if id == JsonStringify { "@json.stringify" } else { "@json.pretty" };
            let v = args.remove(0);
            let text = crate::json::stringify(&v, id == JsonPretty)
                .map_err(|m| RuntimeError::Json { function: name, message: m })?;
            Value::String(Rc::new(text))
        }
        JsonGet => {
            let (t, p) = (as_string(args.remove(0)), as_string(args.remove(0)));
            let r = crate::json::get(&t, &p).map_err(|m| RuntimeError::Json { function: "@json.get", message: m })?;
            Value::String(Rc::new(r))
        }
        JsonHas => {
            let (t, p) = (as_string(args.remove(0)), as_string(args.remove(0)));
            Value::Bool(crate::json::has(&t, &p).map_err(|m| RuntimeError::Json { function: "@json.has", message: m })?)
        }
        JsonType => {
            let (t, p) = (as_string(args.remove(0)), as_string(args.remove(0)));
            let r = crate::json::type_of(&t, &p).map_err(|m| RuntimeError::Json { function: "@json.type", message: m })?;
            Value::String(Rc::new(r.to_string()))
        }
        JsonLen => {
            let (t, p) = (as_string(args.remove(0)), as_string(args.remove(0)));
            Value::Int(crate::json::len(&t, &p).map_err(|m| RuntimeError::Json { function: "@json.len", message: m })? as i64)
        }
        JsonKeys => {
            let (t, p) = (as_string(args.remove(0)), as_string(args.remove(0)));
            let keys = crate::json::keys(&t, &p).map_err(|m| RuntimeError::Json { function: "@json.keys", message: m })?;
            let items: Vec<Value> = keys.into_iter().map(|k| Value::String(Rc::new(k))).collect();
            Value::Array(Rc::new(std::cell::RefCell::new(items)))
        }
        JsonValid => {
            let t = as_string(args.remove(0));
            Value::Bool(crate::json::valid(&t))
        }
        JsonSet => {
            let (t, p) = (as_string(args.remove(0)), as_string(args.remove(0)));
            let v = args.remove(0);
            let r = crate::json::set(&t, &p, &v).map_err(|m| RuntimeError::Json { function: "@json.set", message: m })?;
            Value::String(Rc::new(r))
        }
        // ---- @file (the disk, the disk, the disk) ----
        FileRead => {
            let p = as_string(args.remove(0));
            let text = std::fs::read_to_string(p.as_str()).map_err(|e| io_error("@file.read", &p, e))?;
            Value::String(Rc::new(text))
        }
        FileWrite => {
            let p = as_string(args.remove(0));
            let text = as_string(args.remove(0));
            std::fs::write(p.as_str(), text.as_bytes()).map_err(|e| io_error("@file.write", &p, e))?;
            Value::Void
        }
        FileAppend => {
            let p = as_string(args.remove(0));
            let text = as_string(args.remove(0));
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(p.as_str())
                .map_err(|e| io_error("@file.append", &p, e))?;
            file.write_all(text.as_bytes()).map_err(|e| io_error("@file.append", &p, e))?;
            Value::Void
        }
        FileLines => {
            let p = as_string(args.remove(0));
            let text = std::fs::read_to_string(p.as_str()).map_err(|e| io_error("@file.lines", &p, e))?;
            string_array(text.lines().map(String::from))
        }
        FileExists => Value::Bool(std::path::Path::new(as_string(args.remove(0)).as_str()).exists()),
        FileIsDir => Value::Bool(std::path::Path::new(as_string(args.remove(0)).as_str()).is_dir()),
        FileList => {
            let p = as_string(args.remove(0));
            let mut names: Vec<String> = Vec::new();
            for entry in std::fs::read_dir(p.as_str()).map_err(|e| io_error("@file.list", &p, e))? {
                let entry = entry.map_err(|e| io_error("@file.list", &p, e))?;
                names.push(entry.file_name().to_string_lossy().into_owned());
            }
            names.sort();
            string_array(names.into_iter())
        }
        FileMkdir => {
            let p = as_string(args.remove(0));
            std::fs::create_dir_all(p.as_str()).map_err(|e| io_error("@file.mkdir", &p, e))?;
            Value::Void
        }
        // a file, or an EMPTY directory. never recursive. no thanks.
        FileDelete => {
            let p = as_string(args.remove(0));
            let path = std::path::Path::new(p.as_str());
            let done = if path.is_dir() { std::fs::remove_dir(path) } else { std::fs::remove_file(path) };
            done.map_err(|e| io_error("@file.delete", &p, e))?;
            Value::Void
        }
        FileCopy => {
            let from = as_string(args.remove(0));
            let to = as_string(args.remove(0));
            std::fs::copy(from.as_str(), to.as_str()).map_err(|e| io_error("@file.copy", &from, e))?;
            Value::Void
        }
        FileRename => {
            let from = as_string(args.remove(0));
            let to = as_string(args.remove(0));
            std::fs::rename(from.as_str(), to.as_str()).map_err(|e| io_error("@file.rename", &from, e))?;
            Value::Void
        }
        // a file or a whole directory tree. refuses a filesystem root. we're not monsters.
        FileDeleteAll => {
            let p = as_string(args.remove(0));
            let path = std::path::Path::new(p.as_str());
            if let Ok(real) = std::fs::canonicalize(path) {
                if real.parent().is_none() {
                    return Err(RuntimeError::Io {
                        function: "@file.deleteAll",
                        path: p.to_string(),
                        reason: "refusing to delete a filesystem root".to_string(),
                    });
                }
            }
            let done = if path.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) };
            done.map_err(|e| io_error("@file.deleteAll", &p, e))?;
            Value::Void
        }
        FileSize => {
            let p = as_string(args.remove(0));
            let meta = std::fs::metadata(p.as_str()).map_err(|e| io_error("@file.size", &p, e))?;
            if meta.is_dir() {
                return Err(RuntimeError::Io { function: "@file.size", path: p.to_string(), reason: "is a directory".to_string() });
            }
            Value::Int(meta.len() as i64)
        }
        FileModified | FileModifiedDate => {
            let p = as_string(args.remove(0));
            let name = if matches!(id, FileModified) { "@file.modified" } else { "@file.modifiedDate" };
            let when = std::fs::metadata(p.as_str()).and_then(|m| m.modified()).map_err(|e| io_error(name, &p, e))?;
            let millis = match when.duration_since(UNIX_EPOCH) {
                Ok(d) => d.as_millis() as i64,
                Err(e) => -(e.duration().as_millis() as i64),
            };
            if matches!(id, FileModified) { Value::Int(millis) } else { Value::String(Rc::new(iso_utc(millis.div_euclid(1000)))) }
        }
        FileReadBytes => {
            let p = as_string(args.remove(0));
            let bytes = std::fs::read(p.as_str()).map_err(|e| io_error("@file.readBytes", &p, e))?;
            if bytes.len() > MAX_RANGE_LEN {
                return Err(RuntimeError::RangeTooLarge { len: bytes.len() as u128, max: MAX_RANGE_LEN });
            }
            Value::Array(Rc::new(std::cell::RefCell::new(bytes.into_iter().map(|b| Value::Int(b as i64)).collect())))
        }
        FileWriteBytes => {
            let p = as_string(args.remove(0));
            let items = as_array(args.remove(0));
            let mut bytes: Vec<u8> = Vec::new();
            for (index, v) in items.borrow().iter().enumerate() {
                let value = as_int(v.clone());
                // checked BEFORE touching the disk. a bad item must not leave a half-written file
                match u8::try_from(value) {
                    Ok(b) => bytes.push(b),
                    Err(_) => return Err(RuntimeError::InvalidByte { index, value }),
                }
            }
            std::fs::write(p.as_str(), &bytes).map_err(|e| io_error("@file.writeBytes", &p, e))?;
            Value::Void
        }
        FileJoin => {
            let a = as_string(args.remove(0));
            let b = as_string(args.remove(0));
            Value::String(Rc::new(std::path::Path::new(a.as_str()).join(b.as_str()).to_string_lossy().into_owned()))
        }

        // ---- @sys (the OS is not our friend) ----
        SysArgs => string_array(PROGRAM_ARGS.get().cloned().unwrap_or_default().into_iter()),
        SysExit => return Err(RuntimeError::Exit { code: i32::try_from(as_int(args.remove(0))).unwrap_or(1) }),
        SysEnv => {
            let name = as_string(args.remove(0));
            let fallback = as_string(args.remove(0));
            match std::env::var(name.as_str()) {
                Ok(v) => Value::String(Rc::new(v)),
                Err(_) => Value::String(fallback),
            }
        }
        SysPlatform => Value::String(Rc::new(std::env::consts::OS.to_string())),
        SysCwd => {
            let dir = std::env::current_dir().map_err(|e| io_error("@sys.cwd", "", e))?;
            Value::String(Rc::new(dir.to_string_lossy().into_owned()))
        }
        SysClear => {
            enable_ansi_once();
            // clear screen, clear scrollback, cursor home. (buffered stdout, flushed right after)
            write_out_text("\x1b[2J\x1b[3J\x1b[H")?;
            flush_stdout();
            Value::Void
        }
        SysReadLine => Value::String(Rc::new(read_stdin_line("@sys.readLine")?)),
        // @ps.run / @ps.runFile build a PsResult, which needs the program. the VM does those.
        // `t.wait()` is handled in the VM loop (it may re-throw a task error). never gets here.
        TaskWait => unreachable!("@task.wait is handled by the VM loop"),
        TaskAll => unreachable!("@task.all is handled by the VM loop"),
        PsRun | PsRunFile => {
            return Err(RuntimeError::Process { function: "@ps.run", reason: "internal: @ps.run is run by the VM".to_string() })
        }
        PsVersion => Value::String(Rc::new(crate::powershell::version()?)),
        PsIsAvailable => Value::Bool(crate::powershell::is_available()),
        SysReadAll => {
            flush_stdout();
            let mut all = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut all).map_err(|e| io_error("@sys.readAll", "", e))?;
            Value::String(Rc::new(all))
        }

        StrLastIndexOf => {
            let s = as_string(args.remove(0));
            let needle = as_string(args.remove(0));
            Value::Int(s.rfind(needle.as_str()).map(|byte| s[..byte].chars().count() as i64).unwrap_or(-1))
        }
        TrimStart => Value::String(Rc::new(as_string(args.remove(0)).trim_start().to_string())),
        TrimEnd => Value::String(Rc::new(as_string(args.remove(0)).trim_end().to_string())),
        PadStart | PadStartFill | PadEnd | PadEndFill => {
            let s = as_string(args.remove(0));
            let width = as_int(args.remove(0));
            let fill = if matches!(id, PadStartFill | PadEndFill) { as_string(args.remove(0)) } else { Rc::new(" ".to_string()) };
            let len = s.chars().count();
            if width <= len as i64 || fill.is_empty() {
                Value::String(s)
            } else {
                if width as u128 > MAX_RANGE_LEN as u128 {
                    return Err(RuntimeError::RangeTooLarge { len: width as u128, max: MAX_RANGE_LEN });
                }
                let padding: String = fill.chars().cycle().take(width as usize - len).collect();
                let at_start = matches!(id, PadStart | PadStartFill);
                Value::String(Rc::new(if at_start { format!("{}{}", padding, s) } else { format!("{}{}", s, padding) }))
            }
        }
        Repeat => {
            let s = as_string(args.remove(0));
            let n = as_int(args.remove(0)).max(0) as usize;
            Value::String(Rc::new(s.repeat(n)))
        }
        IntToString => Value::String(Rc::new(as_int(args.remove(0)).to_string())),
        FloatToString => Value::String(Rc::new(crate::value::format_float(as_float(args.remove(0))))),
        // the variant's name (`Value`'s Display prints that, so the name is the name)
        EnumToString => Value::String(Rc::new(args.remove(0).to_string())),
        AnyToString => Value::String(Rc::new(args.remove(0).to_string())),
        BoolToString => Value::String(Rc::new(match args.remove(0) {
            Value::Bool(b) => b.to_string(),
            other => panic!("VM bug: builtin expected bool, got {}", other.type_name()),
        })),

        // ---- arrays (the big one) ----
        ArrayLenBuiltin => Value::Int(as_array(args.remove(0)).borrow().len() as i64),
        ArrayPush => {
            let arr = as_array(args.remove(0));
            let item = args.remove(0);
            arr.borrow_mut().push(item);
            Value::Void
        }
        // ---- maps (hashmaps: we're proud, a little) ----
        MapHas => {
            let map = as_map(args.remove(0));
            let found = map.borrow().contains(&args[0]);
            Value::Bool(found)
        }
        MapGet => {
            let map = as_map(args.remove(0));
            let found = map.borrow().get(&args[0]).cloned();
            found.unwrap_or_else(|| args.remove(1))
        }
        MapRemove => {
            let map = as_map(args.remove(0));
            let removed = map.borrow_mut().remove(&args[0]);
            Value::Bool(removed)
        }
        MapLen => Value::Int(as_map(args.remove(0)).borrow().len() as i64),
        MapKeys => {
            let map = as_map(args.remove(0));
            let keys: Vec<Value> = map.borrow().entries().iter().map(|(k, _)| k.clone()).collect();
            Value::Array(Rc::new(std::cell::RefCell::new(keys)))
        }
        MapKeysOf => {
            let map = as_map(args.remove(0));
            let needle = args.remove(0);
            let keys: Vec<Value> = map
                .borrow()
                .entries()
                .iter()
                .filter(|(_, v)| v.structurally_equal(&needle))
                .map(|(k, _)| k.clone())
                .collect();
            Value::Array(Rc::new(std::cell::RefCell::new(keys)))
        }
        MapValues => {
            let map = as_map(args.remove(0));
            let values: Vec<Value> = map.borrow().entries().iter().map(|(_, v)| v.clone()).collect();
            Value::Array(Rc::new(std::cell::RefCell::new(values)))
        }
        ArrayPop => {
            let arr = as_array(args.remove(0));
            let popped = arr.borrow_mut().pop();
            match popped {
                Some(v) => v,
                None => return Err(RuntimeError::EmptyArray { function: "@array.pop" }),
            }
        }
        // ---- more @array (it never ends) ----
        ArrayInsert => {
            let arr = as_array(args.remove(0));
            let at = as_int(args.remove(0));
            let item = args.remove(0);
            let len = arr.borrow().len();
            if at < 0 || at as u64 > len as u64 {
                return Err(RuntimeError::IndexOutOfBounds { index: at, len });
            }
            arr.borrow_mut().insert(at as usize, item);
            Value::Void
        }
        ArrayRemove => {
            let arr = as_array(args.remove(0));
            let at = as_int(args.remove(0));
            let len = arr.borrow().len();
            if at < 0 || at as u64 >= len as u64 {
                return Err(RuntimeError::IndexOutOfBounds { index: at, len });
            }
            let removed = arr.borrow_mut().remove(at as usize);
            removed
        }
        ArrayClear => {
            as_array(args.remove(0)).borrow_mut().clear();
            Value::Void
        }
        ArrayFill => {
            let n = as_int(args.remove(0));
            let item = args.remove(0);
            if n < 0 {
                return Err(RuntimeError::NegativeCount { function: "@array.fill", count: n });
            }
            if n as u128 > MAX_RANGE_LEN as u128 {
                return Err(RuntimeError::RangeTooLarge { len: n as u128, max: MAX_RANGE_LEN });
            }
            Value::Array(Rc::new(std::cell::RefCell::new(vec![item; n as usize])))
        }
        ArrayConcat => {
            let a = as_array(args.remove(0));
            let b = as_array(args.remove(0));
            let mut joined: Vec<Value> = a.borrow().clone();
            joined.extend(b.borrow().iter().cloned());
            Value::Array(Rc::new(std::cell::RefCell::new(joined)))
        }
        ArraySumInt => {
            let arr = as_array(args.remove(0));
            let mut total = 0i64;
            for v in arr.borrow().iter() {
                total = crate::arith::add(total, as_int(v.clone()))?;
            }
            Value::Int(total)
        }
        ArraySumFloat => {
            let arr = as_array(args.remove(0));
            let total = arr.borrow().iter().fold(0.0f64, |acc, v| acc + as_float(v.clone()));
            Value::Float(total)
        }
        ArrayContains => {
            let arr = as_array(args.remove(0));
            let needle = args.remove(0);
            let found = arr.borrow().iter().any(|v| v.structurally_equal(&needle));
            Value::Bool(found)
        }
        ArrayIndexOf => {
            let arr = as_array(args.remove(0));
            let needle = args.remove(0);
            let idx = arr.borrow().iter().position(|v| v.structurally_equal(&needle)).map(|i| i as i64).unwrap_or(-1);
            Value::Int(idx)
        }
        ArraySlice => {
            let arr = as_array(args.remove(0));
            let start = as_int(args.remove(0));
            let end = as_int(args.remove(0));
            let borrowed = arr.borrow();
            let len = borrowed.len() as i64;
            if start < 0 || end < start || end > len {
                return Err(RuntimeError::IndexOutOfBounds { index: end, len: borrowed.len() });
            }
            let slice = borrowed[start as usize..end as usize].to_vec();
            Value::Array(Rc::new(std::cell::RefCell::new(slice)))
        }
        ArrayRange2 => {
            let start = as_int(args.remove(0));
            let end = as_int(args.remove(0));
            range_array(start, end, 1)?
        }
        ArrayRange3 => {
            let start = as_int(args.remove(0));
            let end = as_int(args.remove(0));
            let step = as_int(args.remove(0));
            range_array(start, end, step)?
        }
        RangeStepCheck => {
            let step = as_int(args.remove(0));
            if step == 0 {
                return Err(RuntimeError::ZeroRangeStep);
            }
            Value::Int(step)
        }
        ArrayReverse => {
            let arr = as_array(args.remove(0));
            arr.borrow_mut().reverse();
            Value::Array(arr)
        }
        ArraySort => {
            let arr = as_array(args.remove(0));
            arr.borrow_mut().sort_by(|a, b| compare_values(a, b));
            Value::Array(arr)
        }
        ArrayJoin => {
            let arr = as_array(args.remove(0));
            let sep = as_string(args.remove(0));
            let joined = arr.borrow().iter().map(|v| v.to_string()).collect::<Vec<_>>().join(&sep);
            Value::String(Rc::new(joined))
        }

        // ---- math (floats, the untrustworthy kind) ----
        Abs => Value::Int(crate::arith::abs(as_int(args.remove(0)))?),
        AbsFloat => Value::Float(as_float(args.remove(0)).abs()),
        Floor => Value::Int(as_float(args.remove(0)).floor() as i64),
        Ceil => Value::Int(as_float(args.remove(0)).ceil() as i64),
        Round => Value::Int(as_float(args.remove(0)).round() as i64),
        Sqrt => Value::Float(as_float(args.remove(0)).sqrt()),
        PowFloat => {
            let base = as_float(args.remove(0));
            let exp = as_float(args.remove(0));
            Value::Float(base.powf(exp))
        }
        MinInt => Value::Int(as_int(args.remove(0)).min(as_int(args.remove(0)))),
        MaxInt => Value::Int(as_int(args.remove(0)).max(as_int(args.remove(0)))),
        MinFloat => Value::Float(as_float(args.remove(0)).min(as_float(args.remove(0)))),
        MaxFloat => Value::Float(as_float(args.remove(0)).max(as_float(args.remove(0)))),
        Random => Value::Float(pseudo_random()),
        // ---- more @math (we're so back) ----
        Sin => Value::Float(as_float(args.remove(0)).sin()),
        Cos => Value::Float(as_float(args.remove(0)).cos()),
        Tan => Value::Float(as_float(args.remove(0)).tan()),
        Asin => Value::Float(as_float(args.remove(0)).asin()),
        Acos => Value::Float(as_float(args.remove(0)).acos()),
        Atan => Value::Float(as_float(args.remove(0)).atan()),
        Atan2 => {
            let y = as_float(args.remove(0));
            let x = as_float(args.remove(0));
            Value::Float(y.atan2(x))
        }
        Exp => Value::Float(as_float(args.remove(0)).exp()),
        Log => Value::Float(as_float(args.remove(0)).ln()),
        Log2 => Value::Float(as_float(args.remove(0)).log2()),
        Log10 => Value::Float(as_float(args.remove(0)).log10()),
        ToRadians => Value::Float(as_float(args.remove(0)).to_radians()),
        ToDegrees => Value::Float(as_float(args.remove(0)).to_degrees()),
        SignInt => Value::Int(as_int(args.remove(0)).signum()),
        SignFloat => {
            let x = as_float(args.remove(0));
            Value::Int(if x > 0.0 { 1 } else if x < 0.0 { -1 } else { 0 })
        }
        ClampInt => {
            let x = as_int(args.remove(0));
            let low = as_int(args.remove(0));
            let high = as_int(args.remove(0));
            if low > high {
                return Err(RuntimeError::ClampBounds { low: low.to_string(), high: high.to_string() });
            }
            Value::Int(x.max(low).min(high))
        }
        ClampFloat => {
            let x = as_float(args.remove(0));
            let low = as_float(args.remove(0));
            let high = as_float(args.remove(0));
            if low > high {
                return Err(RuntimeError::ClampBounds { low: low.to_string(), high: high.to_string() });
            }
            // written out (not `f64::clamp`, which panics on a NaN bound. thanks rust).
            Value::Float(if x < low { low } else if x > high { high } else { x })
        }
        TauDecimals => Value::Float(round_decimals(std::f64::consts::TAU, as_int(args.remove(0)), "@math.tau")?),
        TauDefault => Value::Float(round_decimals(std::f64::consts::TAU, DEFAULT_DECIMALS, "@math.tau")?),
        PiDecimals => Value::Float(round_decimals(std::f64::consts::PI, as_int(args.remove(0)), "@math.pi")?),
        EDecimals => Value::Float(round_decimals(std::f64::consts::E, as_int(args.remove(0)), "@math.e")?),
        PiDefault => Value::Float(round_decimals(std::f64::consts::PI, DEFAULT_DECIMALS, "@math.pi")?),
        EDefault => Value::Float(round_decimals(std::f64::consts::E, DEFAULT_DECIMALS, "@math.e")?),

        // ---- @into (conversions, which are always a lie) ----
        IntToFloat => Value::Float(as_int(args.remove(0)) as f64),
        FloatToInt => Value::Int(as_float(args.remove(0)) as i64),
        ParseInt => {
            let s = as_string(args.remove(0));
            s.trim().parse::<i64>().map(Value::Int).map_err(|_| RuntimeError::InvalidNumberFormat {
                input: s.to_string(),
                target_type: "int",
            })?
        }
        ParseFloat => {
            let s = as_string(args.remove(0));
            s.trim().parse::<f64>().map(Value::Float).map_err(|_| RuntimeError::InvalidNumberFormat {
                input: s.to_string(),
                target_type: "float",
            })?
        }

        // ---- time (the clock lies too) ----
        // @time.now(): current UTC date-time, ISO-8601 string. the format nobody remembers.
        TimeNow => Value::String(Rc::new(iso_utc(unix_millis() / 1000))),
        // @time.unix(): whole ms since 1970-01-01 UTC. wall clock, so it can jump around.
        TimeUnix => Value::Int(unix_millis()),
        // @time.elapsed(): time since the program started (monotonic clock),
        // already formatted in whole units that adapt to size: "45ms",
        // "3s 250ms", "1h 2m". 
        TimeElapsed => Value::String(Rc::new(format_duration(program_start().elapsed().as_nanos()))),
        // @time.sleep(ms): blocks the script right here. like a wait(), but worse.
        // `@time.sleep` is handled in the VM loop, where it can give the baton away. never gets here.
        TimeSleep => unreachable!("@time.sleep is handled by the VM loop"),
    })
}

/// `nanos` as whole-number units that adapt to the size. like Rust's own
/// `Duration` output, but integer-only. the largest unit that fits at least once,
/// plus the next smaller unit if it's not zero (that's the whole precision
/// shown, so 2h 3m 4s reads `2h 3m`). units: h m s ms µs ns. no `ps`, because
/// the monotonic clock ticks in ns, so ps would always be 0. don't add it.
/// (it would always be 0. I checked. twice.)
fn format_duration(nanos: u128) -> String {
    const UNITS: [(u128, &str); 6] = [
        (3_600_000_000_000, "h"),
        (60_000_000_000, "m"),
        (1_000_000_000, "s"),
        (1_000_000, "ms"),
        (1_000, "\u{b5}s"),
        (1, "ns"),
    ];
    for (i, (size, name)) in UNITS.iter().enumerate() {
        if nanos >= *size {
            let mut out = format!("{}{}", nanos / size, name);
            if let Some((next_size, next_name)) = UNITS.get(i + 1) {
                let next = (nanos % size) / next_size;
                if next > 0 {
                    out.push_str(&format!(" {}{}", next, next_name));
                }
            }
            return out;
        }
    }
    "0ns".to_string()
}

/// decimals `@math.pi()` / `@math.e()` use with no argument. magic number,
const DEFAULT_DECIMALS: i64 = 2;
/// an f64 holds ~15-17 significant digits, so more decimals than that
/// adds nothing real. bigger requests get clamped. honesty first.
const MAX_DECIMALS: i64 = 15;

/// max elements `@array.range` will build. bigger = runtime error, not an
/// OOM abort (which nothing could catch). crash politely.
pub const MAX_RANGE_LEN: usize = 20_000_000;

/// `@array.range(start, end, step)`: `start, start+step, ...` while strictly
/// before `end` (after it, for a negative step). `end` is EXCLUSIVE. works in
/// `i128`, so the i64 extremes can't overflow while counting. math is hard.
fn range_array(start: i64, end: i64, step: i64) -> Result<Value, RuntimeError> {
    if step == 0 {
        return Err(RuntimeError::ZeroRangeStep);
    }
    let (start, end, step) = (start as i128, end as i128, step as i128);
    let distance = if step > 0 { end - start } else { start - end };
    let len: u128 = if distance <= 0 {
        0
    } else {
        let s = step.unsigned_abs();
        (distance as u128 + s - 1) / s
    };
    if len > MAX_RANGE_LEN as u128 {
        return Err(RuntimeError::RangeTooLarge { len, max: MAX_RANGE_LEN });
    }
    let items: Vec<Value> = (0..len as i128).map(|k| Value::Int((start + k * step) as i64)).collect();
    Ok(Value::Array(Rc::new(std::cell::RefCell::new(items))))
}

/// `value` rounded (half away from zero, on the decimal) to
/// `decimals` places, e.g. `pi` with 4 → 3.1416. goes through the decimal
/// formatter instead of `(x * 10^n).round() / 10^n`, so no float error
/// sneaks in from the scaling. floats are liars.
fn round_decimals(value: f64, decimals: i64, function: &'static str) -> Result<f64, RuntimeError> {
    if decimals < 0 {
        return Err(RuntimeError::NegativeDecimals { function, decimals });
    }
    let n = decimals.min(MAX_DECIMALS) as usize;
    Ok(format!("{:.*}", n, value).parse::<f64>().expect("a formatted float always re-parses"))
}

static PROGRAM_ARGS: OnceLock<Vec<String>> = OnceLock::new();

/// the args `@sys.args()` returns (what comes after `--` on the command line).
/// set once, before the program runs. later calls do nothing.
pub fn set_program_args(args: Vec<String>) {
    let _ = PROGRAM_ARGS.set(args);
}

fn io_error(function: &'static str, path: &str, e: std::io::Error) -> RuntimeError {
    use std::io::ErrorKind;
    let reason = match e.kind() {
        ErrorKind::NotFound => "no such file or directory".to_string(),
        ErrorKind::PermissionDenied => "permission denied".to_string(),
        ErrorKind::AlreadyExists => "already exists".to_string(),
        ErrorKind::InvalidData => "not valid UTF-8 text".to_string(),
        _ => e.to_string(),
    };
    RuntimeError::Io { function, path: path.to_string(), reason }
}

fn string_array(items: impl Iterator<Item = String>) -> Value {
    let values: Vec<Value> = items.map(|s| Value::String(Rc::new(s))).collect();
    Value::Array(Rc::new(std::cell::RefCell::new(values)))
}

static PROGRAM_START: OnceLock<Instant> = OnceLock::new();

/// fixes the instant `@time.elapsed()` counts from. `Vm::new` calls this so the
/// clock starts when the program does (not at the first elapsed call).
/// later calls do nothing.
pub fn mark_program_start() {
    PROGRAM_START.get_or_init(Instant::now);
}

fn program_start() -> Instant {
    *PROGRAM_START.get_or_init(Instant::now)
}

/// whole ms since the Unix epoch. if the system clock is set before 1970,
/// you get a negative number instead of a panic. nice of it.
fn unix_millis() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_millis() as i64,
        Err(e) => -(e.duration().as_millis() as i64),
    }
}

/// `secs` since the Unix epoch → `YYYY-MM-DDTHH:MM:SSZ` (UTC). uses
/// Howard Hinnant's days-to-civil algorithm. no date crate. respect.
fn iso_utc(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);

    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };

    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, m, d, h, mi, s)
}

/// ordering for `sort`. spg has no user comparators yet, so this
/// covers exactly what array builtins accept: numbers
/// compare as numbers, strings lexicographically, bools false-before-true.
/// mixed-type arrays can't exist after typecheck (arrays are homogeneous),
/// so every comparison here is same-variant. no weird cases. (please.)
fn compare_values(a: &Value, b: &Value) -> std::cmp::Ordering {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => x.cmp(y),
        (Value::Float(x), Value::Float(y)) => x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal),
        (Value::String(x), Value::String(y)) => x.cmp(y),
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        _ => panic!("VM bug: sort on a non-homogeneous or non-orderable array"),
    }
}

/// a tiny xorshift PRNG seeded from the system clock. no crate,
/// re-seeded on every call (spg has no global VM state to stash a seed in
/// yet). fine for `random()`'s real job (game jitter, not
/// crypto or serious simulation), but calls
/// in the same ns tick could in theory repeat. acceptable for v1.0.
/// revisit if anyone ever notices. nobody will. probably.
fn pseudo_random() -> f64 {
    let seed = SystemTime::now().duration_since(UNIX_EPOCH).expect("system clock is before the Unix epoch").as_nanos()
        as u64;
    let mut x = seed ^ 0x9E3779B97F4A7C15;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    (x as f64 / u64::MAX as f64).abs().fract()
}

#[cfg(test)]
mod tests {
    use super::format_duration as fd;

    #[test]
    fn duration_picks_the_unit_that_fits() {
        assert_eq!(fd(0), "0ns");
        assert_eq!(fd(1), "1ns");
        assert_eq!(fd(999), "999ns");
        assert_eq!(fd(1_000), "1\u{b5}s");
        assert_eq!(fd(45_000_000), "45ms");
        assert_eq!(fd(3_000_000_000), "3s");
        assert_eq!(fd(60_000_000_000), "1m");
        assert_eq!(fd(3_600_000_000_000), "1h");
        // far beyond a day stays in hours (there is no "d". don't ask for days.)
        assert_eq!(fd(1_700_000_000_000 * 1_000_000), "472222h 13m");
    }

    #[test]
    fn duration_shows_the_next_unit_only_when_it_is_not_zero() {
        assert_eq!(fd(1_500), "1\u{b5}s 500ns");
        assert_eq!(fd(2_000_300), "2ms"); // 0 \u{b5}s in between: nothing appended
        assert_eq!(fd(3_250_000_000), "3s 250ms");
        assert_eq!(fd(90_000_000_000), "1m 30s");
        assert_eq!(fd(3_725_000_000_000), "1h 2m");
        // two units only: seconds get dropped. 2h 3m 4s becomes 2h 3m. sorry seconds.
        assert_eq!(fd(7_384_000_000_000), "2h 3m");
        assert_eq!(fd(3_605_000_000_000), "1h"); // 1h 0m 5s
    }
}
