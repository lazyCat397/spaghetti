//! ui-core: the core of `install #ui`. window, loop, keyboard and rectangles.
//!
//! nothing here depends on the window library. the real backend is behind the `minifb`
//! feature. the tests use the `NullBackend`.

pub mod backend;
pub mod color;
pub mod null;
pub mod session;

#[cfg(feature = "minifb")]
pub mod minifb_backend;

pub use backend::{key_from_name, Backend, Key};
pub use session::{Session, State, MAX_DT};
