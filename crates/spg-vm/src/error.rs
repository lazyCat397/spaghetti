//! Runtime errors the VM can raise. Distinct from `spg_typecheck::TypeError`
//! (compile-time) — everything here should be UNREACHABLE for a program
//! that passed type-checking and compiled successfully; these exist as a
//! defensive last line, not as user-facing diagnostics for normal
//! programs. The one exception is `UnhandledException`, which is a real
//! runtime condition (an exception raised with no active `try/catch`
//! covering it) rather than a VM bug.

use std::fmt;

#[derive(Debug, Clone)]
pub enum RuntimeError {
    /// an OpCode expected a certain Value variant on the stack (e.g.
    /// AddInt expects two Ints) and found something else — should be
    /// impossible post-typecheck; the type name found is kept for
    /// whatever debugging is still useful if this ever fires
    TypeMismatch { expected: &'static str, found: &'static str },
    /// popped an empty operand stack — compiler bug (stack imbalance)
    StackUnderflow,
    /// array index out of bounds at runtime (arrays are dynamically
    /// sized — this is NOT a type error, it's a real runtime condition
    /// even in a fully well-typed program, e.g. `arr[i]` where `i` came
    /// from user input or a computed offset)
    IndexOutOfBounds { index: i64, len: usize },
    /// `@sys.exit(code)`: end the program now. Never catchable; the CLI turns it into the process exit code.
    Exit { code: i32 },
    /// a failed `@file.*` / `@sys.*` operation (missing file, no permission...); catchable
    Io { function: &'static str, path: String, reason: String },
    /// `readLine()` / `@sys.readLine()` with nothing left on stdin; `function` names the caller
    EndOfInput { function: &'static str },
    /// `@file.writeBytes(p, [..., 300])`: a byte goes from 0 to 255
    InvalidByte { index: usize, value: i64 },
    /// `@array.pop` on an empty array
    EmptyArray { function: &'static str },
    /// `@array.fill(n, x)` with a negative `n`
    NegativeCount { function: &'static str, count: i64 },
    /// `@math.clamp(x, low, high)` with `low > high`
    ClampBounds { low: String, high: String },
    /// `m[k]` (or `m[k] += v`) with a key the map does not have; `key` is how the
    /// key prints (strings quoted). `@map.has` / `@map.get(m, k, fallback)` avoid it.
    KeyNotFound { key: String },
    /// division or modulo by zero — same category as index-out-of-bounds:
    /// a real runtime condition, not a typechecker-catchable bug
    DivisionByZero,
    /// an exception was thrown (via the `Throw` opcode) with no active
    /// `try/catch` handler anywhere in the call stack to catch it —
    /// this is a genuine "the program's own logic failed" condition,
    /// not a VM bug
    UnhandledException(String),
    /// called a function index that doesn't exist in the program —
    /// compiler bug, should be impossible post-typecheck
    InvalidFunctionIndex(usize),
    /// `@into.int`/`@into.float` on a string that isn't valid syntax for
    /// the target type — a real runtime condition (the string is user or
    /// external data, its contents aren't known at compile time), not a
    /// VM bug
    InvalidNumberFormat { input: String, target_type: &'static str },
    /// `@time.sleep(ms)` with a negative duration — a real runtime condition
    /// (the argument can be computed), not a VM bug
    NegativeSleep(i64),
    /// `@math.pi(n)` / `@math.e(n)` with a negative number of decimals
    NegativeDecimals { function: &'static str, decimals: i64 },
    /// a range with step 0 (`@array.range(1, 4, 0)`, or `for i in 1..4 step n`
    /// with `n == 0`): it could never reach its end
    ZeroRangeStep,
    /// `@array.range` asked to build more elements than `MAX_RANGE_LEN`
    RangeTooLarge { len: u128, max: usize },
    /// `#> strict`: an int operation whose result does not fit in 64 bits
    IntegerOverflow { op: &'static str },
    /// more nested calls than `vm::MAX_CALL_DEPTH` (usually an infinite recursion)
    StackOverflow { limit: usize },
    /// `@json.*`: invalid JSON text, a path that leads nowhere, a value with no JSON form
    Json { function: &'static str, message: String },
    /// `ps.*`: PowerShell is missing or could not be started (catchable)
    Process { function: &'static str, reason: String },
}

impl RuntimeError {
    /// The text a `catch (e)` receives for this error, or `None` when it is not
    /// catchable: a `throw` gives its message, a runtime condition of the program
    /// (division by zero, index out of bounds, missing key, bad number text...)
    /// gives its usual message, and a VM/compiler bug is never caught.
    pub fn catchable_message(&self) -> Option<String> {
        match self {
            RuntimeError::UnhandledException(message) => Some(message.clone()),
            // a `none` where a value was needed is a program error a `catch` can handle
            RuntimeError::TypeMismatch { found: "none", .. } => Some(self.to_string()),
            RuntimeError::TypeMismatch { .. }
            | RuntimeError::StackUnderflow
            | RuntimeError::InvalidFunctionIndex(_)
            // `@sys.exit` is not an error: no `try` may swallow it
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
            RuntimeError::Json { function, message } => write!(f, "{}: {}", function, message),
            RuntimeError::Process { function, reason } => write!(f, "{}: {}", function, reason),
            RuntimeError::StackOverflow { limit } => write!(
                f,
                "stack overflow: more than {} nested calls (is a function calling itself forever?)",
                limit
            ),
            RuntimeError::EmptyArray { function } => write!(f, "{} on an empty array", function),
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
    /// the same fact in the friendly tone (`#> friendly-terminal`)
    pub fn friendly(&self) -> String {
        let hint = match self {
            RuntimeError::IntegerOverflow { .. } => spg_lexer::HINT_OVERFLOW,
            _ => spg_lexer::HINT_RUNTIME,
        };
        spg_lexer::friendly_sentence(&self.to_string(), hint)
    }
}
