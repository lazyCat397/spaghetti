//! spg-vm: bytecode format (`chunk`), runtime values (`value`), runtime
//! errors (`error`), and the interpreter loop itself (`vm`). the good stuff.
//!
//! entry point: `Vm::new(&Program).run()`. that's it. that's the whole crate.

pub mod builtins;
pub mod chunk;
pub mod error;
mod arith;
mod libs;
mod sched;
mod field_ops;
mod index_ops;
pub mod value;
pub mod vm;

pub use builtins::{call_builtin, flush_stdout, flush_stdout_in_panic, seed_random, set_program_args};
pub use chunk::{Chunk, OpCode, Program};
pub use arith::set_strict;
pub use spg_libs::{set_safe_party, take_test_report, TestFailure, TestReport};
pub use error::RuntimeError;
pub use value::{ArrayRef, EnumDef, Value};
pub use vm::{call_depth_limit, set_call_depth_limit, NativeCalls, TraceFrame, Vm, MAX_CALL_DEPTH};
