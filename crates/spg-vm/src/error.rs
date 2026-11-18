//! Runtime errors the VM can raise. not the same as `spg_typecheck::TypeError`
//! (compile-time). almost everything here should be UNREACHABLE for a program
//! that passed the checker and compiled. these exist as a
//! defensive last line, not user-facing errors for normal
//! programs. the one exception is `UnhandledException`, a real
//! runtime condition (an exception thrown with no active `try/catch`
//! covering it). not a VM bug. the program's own fault.

use std::fmt;

#[derive(Debug, Clone)]
pub enum RuntimeError {
    /// an OpCode expected a certain Value variant on the stack (e.g.
    /// AddInt expects two Ints) and found something else. should be
    /// impossible after typecheck. the type name found is kept for
    /// debugging, in case this ever fires. (it will. someday.)
    TypeMismatch { expected: &'static str, found: &'static str },
    /// popped an empty operand stack. compiler bug (stack imbalance). blame the compiler.
    StackUnderflow,
    /// array index out of bounds at runtime (arrays are dynamically
    /// sized. NOT a type error, it's a real runtime condition
    /// even in a perfectly typed program, e.g. `arr[i]` where `i` came
    /// from user input or a computed offset. life is unfair.
    IndexOutOfBounds { index: i64, len: usize },
    /// a write to a `const` value, or to anything reachable from one. it's read-only.
    ReadOnly,
    /// `@sys.exit(code)`: end the program now. never catchable. the CLI turns it into the exit code.
    Exit { code: i32 },
    /// a failed `@file.*` / `@sys.*` op (missing file, no permission...). catchable.
    Io { function: &'static str, path: String, reason: String },
    /// `readLine()` / `@sys.readLine()` with nothing left on stdin. `function` names the caller.
    EndOfInput { function: &'static str },
    /// `@file.writeBytes(p, [..., 300])`: bytes go from 0 to 255. 300 is not a byte. shocking.
    InvalidByte { index: usize, value: i64 },
    /// `@array.pop` on an empty array. pop nothing, get an error. fair.
    EmptyArray { function: &'static str },
    /// a random range whose low bound is above its high bound (`@math.random(20, 1)`)
    EmptyRange { function: &'static str, low: String, high: String },
    /// `@array.fill(n, x)` with a negative `n`. you can't fill -3 things.
    NegativeCount { function: &'static str, count: i64 },
    /// `@math.clamp(x, low, high)` with `low > high`. flipped, bro.
    ClampBounds { low: String, high: String },
    /// `m[k]` (or `m[k] += v`) with a key the map doesn't have. `key` is how the
    /// key prints (strings quoted). `@map.has` / `@map.get(m, k, fallback)` dodge it.
    KeyNotFound { key: String },
    /// division or modulo by zero. same family as index-out-of-bounds:
    /// real runtime condition, not something the checker could catch
    DivisionByZero,
    /// an exception was thrown (`Throw` opcode) with no active
    /// `try/catch` handler anywhere up the call stack. nobody caught it.
    /// a genuine "the program's own logic failed" condition.
    /// not a VM bug. (it's the program. it's always the program.)
    UnhandledException(String),
    /// called a function index that doesn't exist in the program.
    /// compiler bug, should be impossible post-typecheck. yet here we are.
    InvalidFunctionIndex(usize),
    /// `@into.int`/`@into.float` on a string that isn't valid for
    /// the target type. a real runtime condition (the string is user or
    /// external data, we can't know it at compile time), not a
    /// VM bug. users typing garbage is not my fault.
    InvalidNumberFormat { input: String, target_type: &'static str },
    /// `@time.sleep(ms)` with a negative duration. a real runtime condition
    /// (the arg can be computed at runtime), not a VM bug. sleep backwards? no.
    NegativeSleep(i64),
    /// `@math.pi(n)` / `@math.e(n)` with negative decimals. -2 decimals of pi? nope.
    NegativeDecimals { function: &'static str, decimals: i64 },
    /// a range with step 0 (`@array.range(1, 4, 0)`, or `for i in 1..4 step n`
    /// with `n == 0`). it would never reach its end. infinite loop detected, lol.
    ZeroRangeStep,
    /// `@array.range` asked to build more elements than `MAX_RANGE_LEN`. too greedy.
    RangeTooLarge { len: u128, max: usize },
    /// `#> strict`: an int op whose result doesn't fit in 64 bits. you asked for it.
    IntegerOverflow { op: &'static str },
    /// `@into.i8(300)`: the value does not fit the target. `suggest_wrap`: the `w` version exists
    IntoOutOfRange { value: i64, ty: &'static str, lo: i64, hi: i64, suggest_wrap: bool },
    /// a u64 above i64::MAX converted to int: it does not fit
    U64NotInt { value: u64 },
    /// a u64 result left the u64 range under `#> strict`
    UnsignedOverflow { op: &'static str },
    /// a sized int (`i8`, `u8`, `u64`) left its range: an error under `#> strict`, or always for `u64`
    SizedOutOfRange { ty: &'static str, op: &'static str, lo: i64, hi: i64, wraps: bool },
    /// more nested calls than `vm::MAX_CALL_DEPTH` (usually infinite recursion. classic.)
    StackOverflow { limit: usize },
    /// `@json.*`: invalid JSON text, a path to nowhere, a value with no JSON form
    Json { function: &'static str, message: String },
    /// `ps.*`: PowerShell is missing or wouldn't start. catchable. good luck on Linux.
    Process { function: &'static str, reason: String },
}

impl RuntimeError {
    /// the text a `catch (e)` receives for this error, or `None` if it's not
    /// catchable. a `throw` gives its message, a runtime condition of the program
    /// (div by zero, index out of bounds, missing key, bad number text...)
    /// gives its usual message. a VM/compiler bug is never caught. never. don't even try.
    pub fn catchable_message(&self) -> Option<String> {
        match self {
            RuntimeError::UnhandledException(message) => Some(message.clone()),
            // a `none` where a value was needed is a program error a `catch` can handle. fair.
            RuntimeError::TypeMismatch { found: "none", .. } => Some(self.to_string()),
            RuntimeError::TypeMismatch { .. }
            | RuntimeError::StackUnderflow
            | RuntimeError::InvalidFunctionIndex(_)
            // `@sys.exit` is not an error. no `try` gets to swallow it. exit means exit.
            | RuntimeError::Exit { .. } => None,
            other => Some(other.to_string()),
        }
    }
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RuntimeError::TypeMismatch { expected, found: "none" } => write!(
                f,
                "unexpected none: a {} was needed but the value is none (an optional was used without checking it; a global can become none inside a function called in between)",
                expected
            ),
            RuntimeError::TypeMismatch { expected, found } => {
                write!(f, "VM bug: expected {} on the stack, found {}", expected, found)
            }
            RuntimeError::StackUnderflow => write!(f, "VM bug: operand stack underflow"),
            RuntimeError::IndexOutOfBounds { index, len } => {
                write!(f, "index out of bounds: {} (array has {} element(s))", index, len)
            }
            RuntimeError::ReadOnly => write!(f, "cannot modify a const value (it is read-only); use .copy() to change a copy"),
            RuntimeError::Exit { code } => write!(f, "exit({})", code),
            RuntimeError::Io { function, path, reason } => {
                if path.is_empty() {
                    write!(f, "{}: {}", function, reason)
                } else {
                    write!(f, "{}(\"{}\"): {}", function, path, reason)
                }
            }
            RuntimeError::InvalidByte { index, value } => {
                write!(f, "@file.writeBytes: the item at index {} is {}; bytes go from 0 to 255", index, value)
            }
            RuntimeError::EndOfInput { function } => write!(f, "{}: end of input (nothing left on stdin)", function),
            RuntimeError::IntegerOverflow { op } => {
                write!(f, "integer overflow in {}: the result does not fit in an int (64 bits, #> strict)", op)
            }
            RuntimeError::IntoOutOfRange { value, ty, lo, hi, suggest_wrap } => {
                write!(f, "{} does not fit {} (range {}..={})", value, ty, lo, hi)?;
                if *suggest_wrap {
                    write!(f, ": use @into.{}w to wrap it into the range", ty)?;
                }
                Ok(())
            }
            RuntimeError::U64NotInt { value } => write!(
                f,
                "the u64 value {} does not fit int (its largest value is 9223372036854775807)",
                value
            ),
            RuntimeError::UnsignedOverflow { op } => {
                write!(f, "{} overflow in u64: the result does not fit in u64 (#> strict)", op)
            }
            RuntimeError::SizedOutOfRange { ty, op, lo, hi, wraps } => {
                if *wraps {
                    write!(f, "{} overflow in {}: the result is outside {}'s range {}..={} (#> strict)", op, ty, ty, lo, hi)
                } else {
                    write!(
                        f,
                        "{} in {}: the result is outside {}'s range {}..={}; {} does not wrap yet, its range ends at i64::MAX",
                        op, ty, ty, lo, hi, ty
                    )
                }
            }
            RuntimeError::Json { function, message } => write!(f, "{}: {}", function, message),
            RuntimeError::Process { function, reason } => write!(f, "{}: {}", function, reason),
            RuntimeError::StackOverflow { limit } => write!(
                f,
                "stack overflow: more than {} nested calls (is a function calling itself forever?)",
                limit
            ),
            RuntimeError::EmptyArray { function } => write!(f, "{} on an empty array", function),
            RuntimeError::EmptyRange { function, low, high } => {
                write!(f, "{}: the low bound {} is above the high bound {}", function, low, high)
            }
            RuntimeError::NegativeCount { function, count } => {
                write!(f, "{} needs a count of 0 or more, got {}", function, count)
            }
            RuntimeError::ClampBounds { low, high } => {
                write!(f, "@math.clamp: the lower bound ({}) is greater than the upper bound ({})", low, high)
            }
            RuntimeError::KeyNotFound { key } => {
                write!(f, "key not found in map: {} (use @map.has(m, k) or @map.get(m, k, fallback))", key)
            }
            RuntimeError::DivisionByZero => write!(f, "division by zero"),
            RuntimeError::UnhandledException(msg) => write!(f, "unhandled exception: {}", msg),
            RuntimeError::InvalidFunctionIndex(idx) => write!(f, "VM bug: invalid function index {}", idx),
            RuntimeError::InvalidNumberFormat { input, target_type } => {
                write!(f, "cannot parse \"{}\" as {}", input, target_type)
            }
            RuntimeError::NegativeDecimals { function, decimals } => {
                write!(f, "{} expects a non-negative number of decimal places, got {}", function, decimals)
            }
            RuntimeError::ZeroRangeStep => write!(f, "range step cannot be zero"),
            RuntimeError::RangeTooLarge { len, max } => write!(
                f,
                "@array.range would create {} elements (the maximum is {}); use 'for i in a..b' to count without an array",
                len, max
            ),
            RuntimeError::NegativeSleep(ms) => {
                write!(f, "@time.sleep expects a non-negative number of milliseconds, got {}", ms)
            }
        }
    }
}

impl std::error::Error for RuntimeError {}

impl RuntimeError {
    /// the same fact, in the friendly tone (`#> friendly-terminal`). the nice version.
    pub fn friendly(&self) -> String {
        if let RuntimeError::ReadOnly = self {
            return spg_lexer::friendly_sentence("cannot modify a const value (it is read-only)", spg_lexer::HINT_READONLY);
        }
        let hint = match self {
            RuntimeError::IntegerOverflow { .. } => spg_lexer::HINT_OVERFLOW,
            RuntimeError::ReadOnly => spg_lexer::HINT_READONLY,
            _ => spg_lexer::HINT_RUNTIME,
        };
        spg_lexer::friendly_sentence(&self.to_string(), hint)
    }
}
