//! The repository tasks of fresnel. `cargo xtask <command>` runs one.
//!
//! - [`sizes`]: the size limits of the documents and the source files.
//! - [`borders`]: what a crate of the workspace must not depend on.
//! - [`install`]: the macOS bundle and its install.

pub mod borders;
pub mod install;
pub mod sizes;

use std::ffi::OsString;
use std::path::PathBuf;

/// The root of the workspace that a command works on: the folder above the folder of
/// this crate. `from_cargo` is the variable `CARGO_MANIFEST_DIR` of the process.
///
/// `cargo xtask` gives the folder of the crate to the program that it starts, and that
/// folder wins. The path in the binary is for a start with no cargo. A second checkout that
/// builds into the same target folder leaves a binary with its own path, and the size check
/// of this checkout then read the files of the other one and passed.
pub fn workspace_root(from_cargo: Option<OsString>) -> PathBuf {
    from_cargo.map_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")), PathBuf::from).join("..")
}
