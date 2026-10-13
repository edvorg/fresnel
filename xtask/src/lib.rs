//! The repository tasks of fresnel. `cargo xtask <command>` runs one.
//!
//! - [`sizes`]: the size limits of the documents and the source files.
//! - [`borders`]: what a crate of the workspace must not depend on.
//! - [`install`]: the macOS bundle and its install.

pub mod borders;
pub mod install;
pub mod sizes;
