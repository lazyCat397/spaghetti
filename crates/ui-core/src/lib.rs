//! ui-core: the core of `load #ui`. window, loop, keyboard and a scene of rectangles.
//!
//! nothing here depends on the window library. the real backend is behind the `minifb`
//! feature. the tests use the `NullBackend`.

pub mod backend;
pub mod color;
pub mod null;
pub mod picture;
pub mod scene;
pub mod session;
pub mod text;

#[cfg(feature = "minifb")]
pub mod minifb_backend;

pub use backend::{Backend, Button, Key};
pub use picture::Picture;
pub use scene::{Align, Drawn, Image, Label, Scene, Shape, ShapeId};
pub use session::{Session, State, MAX_DT};
