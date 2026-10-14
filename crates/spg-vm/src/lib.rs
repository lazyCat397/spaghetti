//! spg-vm: bytecode format (`chunk`), runtime values (`value`), runtime
//! errors (`error`), and the interpreter loop itself (`vm`).
//!
//! Entry point: `Vm::new(&Program).run()`.

pub mod builtins;
pub mod chunk;
pub mod error;
mod arith;
mod json;
mod powershell;
mod field_ops;
mod index_ops;
pub mod value;
pub mod vm;

pub use builtins::{call_builtin, flush_stdout, flush_stdout_in_panic, set_program_args};
pub use chunk::{Chunk, OpCode, Program};
pub use arith::set_strict;
pub use powershell::set_safe_party;
pub use error::RuntimeError;
pub use value::{ArrayRef, EnumDef, Value};
pub use vm::{TraceFrame, Vm};
