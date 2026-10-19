//! The repository tasks of fresnel. `cargo xtask <command>` runs one.
//!
//! - [`sizes`]: the size limits of the documents and the source files.
//! - [`borders`]: what a crate of the workspace must not depend on.
//! - [`install`]: the macOS bundle and its install. [`windows`]: the program for Windows.
//! - [`notices`]: the license files that go with the bundle and the program for Windows.
//! - [`version`]: the version of a build. `program`: its build. `icon`: the icon of the app.
//! - [`soak`]: the soak with k3s, faults and `fresnel-cli soak`.
//! - [`screenshots`]: the pictures of the README.

pub mod borders;
mod icon;
pub mod install;
pub mod notices;
mod program;
pub mod screenshots;
pub mod sizes;
pub mod soak;
pub mod version;
pub mod windows;

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;

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

/// The build folder of cargo for the workspace at `root`: the variable `CARGO_TARGET_DIR`
/// of the process, which a relative path starts from the root, or `target`.
pub fn target_folder(root: &Path) -> PathBuf {
    root.join(std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| PathBuf::from("target"), PathBuf::from))
}

/// A command of the cargo that runs this task, in the workspace at `root`. It gets no
/// variable that `cargo run` gives this task about its package: ring and the crates above
/// it watch them, so a build with them and the next `cargo test` with none each built
/// those crates again.
pub fn cargo(root: &Path) -> Command {
    let mut cargo = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    cargo.current_dir(root);
    for (name, _) in std::env::vars_os().filter(|(name, _)| of_this_package(name)) {
        cargo.env_remove(name);
    }
    cargo
}

/// True for a variable that cargo gives a program about the package of the program.
fn of_this_package(name: &OsStr) -> bool {
    const NAMES: [&str; 5] =
        ["CARGO_MANIFEST_DIR", "CARGO_MANIFEST_PATH", "CARGO_CRATE_NAME", "CARGO_BIN_NAME", "CARGO_PRIMARY_PACKAGE"];
    name.to_str().is_some_and(|name| name.starts_with("CARGO_PKG_") || NAMES.contains(&name))
}
