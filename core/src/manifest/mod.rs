//! Manifest module — responsible for building the archive blueprint
//! before any compression happens.

pub mod types;
pub mod builder;

#[cfg(not(target_arch = "wasm32"))]
pub mod io;
#[cfg(not(target_arch = "wasm32"))]
pub mod walker;

pub use types::*;
pub use builder::*;

#[cfg(not(target_arch = "wasm32"))]
pub use io::*;
