pub mod builtins;
pub mod error;
pub mod typed_ast;
pub mod checker;

pub use builtins::BuiltinId;
pub use error::TypeError;
pub use typed_ast::{SrcLoc, Ty, TypedBlock, TypedExpr, TypedProgram, TypedStmt};
pub use checker::{resolve_type_expr, type_check, type_check_with};
