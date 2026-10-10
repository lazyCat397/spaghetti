//! Compiler-stage errors. In practice these should be unreachable for any
//! program that already passed `spg-typecheck` — they exist as a defensive
//! layer (e.g. a function call to a name that somehow wasn't registered)
//! rather than user-facing diagnostics, and they're kept in English like
//! every other language-output message per the project's fixed convention.

use std::fmt;

#[derive(Debug, Clone)]
pub enum CompileError {
    /// a call site references a function name the compiler never
    /// registered — should be impossible post-typecheck, but the
    /// compiler never silently emits a broken Call instead of failing
    UnknownFunction(String),
    /// break/continue outside of a loop, or similar structural issue
    /// the typechecker doesn't itself enforce
    InvalidControlFlow(String),
    /// an enum literal names an enum the compiler never registered (post-typecheck: impossible)
    UnknownEnum(String),
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CompileError::UnknownFunction(name) => {
                write!(f, "compiler bug: call to unregistered function '{}'", name)
            }
            CompileError::InvalidControlFlow(msg) => write!(f, "invalid control flow: {}", msg),
            CompileError::UnknownEnum(name) => write!(f, "compiler bug: unregistered enum '{}'", name),
        }
    }
}

impl std::error::Error for CompileError {}

impl CompileError {
    /// the same fact in the friendly tone (`#> friendly-terminal`)
    pub fn friendly(&self) -> String {
        spg_lexer::friendly_sentence(&self.to_string(), spg_lexer::HINT_COMPILE)
    }
}
