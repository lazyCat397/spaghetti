//! spg-lsp: a Language Server Protocol server for Spaghetti, written with
//! nothing but the standard library (JSON-RPC over stdin/stdout, hand-made
//! JSON). It reuses the real front-end — lexer, parser, module resolver,
//! typechecker — so the editor shows exactly the errors `spg` would.

pub mod diag;
pub mod json;
pub mod members;
pub mod server;
pub mod uri;

pub use server::run_stdio;
