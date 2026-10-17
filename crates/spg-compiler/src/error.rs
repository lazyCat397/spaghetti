//! compiler-stage errors. in practice these should be unreachable for any
//! program that already passed `spg-typecheck`. they exist as a defensive
//! layer (e.g. a call to a name that somehow never got registered)
//! not user-facing diagnostics. kept in English like
//! every other output message, per the project's sacred rule.

use std::fmt;

#[derive(Debug, Clone)]
pub enum CompileError {
    /// a call site references a function name the compiler never
    /// registered. should be impossible post-typecheck, but the
    /// compiler never silently emits a broken Call. it fails loudly. good.
    UnknownFunction(String),
    /// break/continue outside of a loop, or a similar structural mess
    /// the typechecker doesn't enforce itself
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
    /// the same fact, friendly tone (`#> friendly-terminal`). the nice version.
    pub fn friendly(&self) -> String {
        spg_lexer::friendly_sentence(&self.to_string(), spg_lexer::HINT_COMPILE)
    }
}
