//! spg-compiler: eats `TypedProgram` (from spg-typecheck) and spits out
//! `spg_vm::chunk::Program`. stack-based bytecode, ready for the VM.
//!
//! entry point: `compile(TypedProgram) -> Result<Program, CompileError>`. the door.

pub mod compiler;
pub mod error;
pub mod peephole;

pub use compiler::{compile, compile_with};
pub use error::CompileError;
