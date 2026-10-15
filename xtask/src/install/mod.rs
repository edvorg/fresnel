//! `cargo xtask install`: builds the release program, makes `Fresnel.app`, signs it and
//! puts it into the applications folder. The earlier bundle stays as a fallback.
//!
//! - [`bundle`]: the content of the bundle and its signature. `icon`: the icon.
//! - [`swap`]: the install by rename. `tool`: a run of a tool of the system.

pub mod bundle;
pub mod icon;
pub mod swap;
mod tool;

use std::io;
use std::path::{Path, PathBuf};

pub use swap::Installed;

/// Where the app goes when the command names no folder.
pub const APPLICATIONS: &str = "/Applications";
const PACKAGE: &str = "fresnel";
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Runs the install into `destination`. `root` is the root of the workspace.
pub fn run(root: &Path, destination: &Path) -> io::Result<Installed> {
    if !cfg!(target_os = "macos") {
        return Err(io::Error::new(io::ErrorKind::Unsupported, "the bundle is for macOS only"));
    }
    let program = build(root)?;
    with_stage(destination, |stage| stage_and_install(&program, stage, destination))
}

/// Runs `work` with a new stage folder and removes the folder after it. The stage is in
/// the destination folder, so it is on the same volume and the install is a rename.
/// An error of the work comes first: it tells more than an error of the removal after it.
pub fn with_stage<T>(destination: &Path, work: impl FnOnce(&Path) -> io::Result<T>) -> io::Result<T> {
    std::fs::create_dir_all(destination)?;
    let stage = destination.join(format!(".fresnel-install-{}", std::process::id()));
    std::fs::create_dir(&stage)?;
    let done = work(&stage);
    let removed = std::fs::remove_dir_all(&stage);
    let done = done?;
    removed?;
    Ok(done)
}

fn stage_and_install(program: &Path, stage: &Path, destination: &Path) -> io::Result<Installed> {
    let app = bundle::assemble(program, VERSION, stage)?;
    bundle::sign(&app)?;
    swap::install(&app, destination)
}

/// Builds the release program with the cargo that runs this task. Returns its path.
fn build(root: &Path) -> io::Result<PathBuf> {
    let status = crate::cargo(root).args(["build", "--release", "--locked", "--package", PACKAGE]).status()?;
    if !status.success() {
        return Err(io::Error::other(format!("the release build failed ({status})")));
    }
    Ok(crate::target_folder(root).join("release").join(PACKAGE))
}
