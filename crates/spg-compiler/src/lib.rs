//! spg-compiler: consumes `TypedProgram` (from spg-typecheck) and emits
//! `spg_vm::chunk::Program` — stack-based bytecode ready for the VM.
//!
//! Entry point: `compile(TypedProgram) -> Result<Program, CompileError>`.

pub mod compiler;
pub mod error;
pub mod peephole;

pub use compiler::{compile, compile_with};
pub use error::CompileError;
