//! Manifest module — defines the archive blueprint data structures
//! and provides pure math for computing byte offsets and fragment indices.
//!
//! This module is fully I/O-agnostic. Filesystem concerns (walking directories,
//! reading/writing JSON files) belong in the CLI crate.

pub mod types;
pub mod builder;

pub use types::*;
pub use builder::*;
