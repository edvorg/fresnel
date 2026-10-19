//! `cargo xtask install`: builds the release program, makes `Fresnel.app` with its license
//! files, signs it and puts it into the applications folder. The earlier bundle stays as a
//! fallback.
//!
//! - [`bundle`]: the content of the bundle and its signature.
//! - [`swap`]: the install by rename. `tool`: a run of a tool of the system.

pub mod bundle;
pub mod swap;
mod tool;

use std::io;
use std::path::Path;

pub use swap::Installed;

use crate::notices::Notices;
use crate::program;
use crate::version::Version;

/// Where the app goes when the command names no folder.
pub const APPLICATIONS: &str = "/Applications";

/// Runs the install of `version` into `destination`. `root` is the root of the workspace. The
/// notices come first: a crate with no license text stops the install before the build.
pub fn run(root: &Path, destination: &Path, version: &Version) -> io::Result<Installed> {
    if !cfg!(target_os = "macos") {
        return Err(io::Error::new(io::ErrorKind::Unsupported, "the bundle is for macOS only"));
    }
    let notices = Notices::of_workspace(root)?;
    let program = program::build(root, version, None)?;
    with_stage(destination, |stage| stage_and_install(&program, &notices, version, stage, destination))
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

fn stage_and_install(
    program: &Path,
    notices: &Notices,
    version: &Version,
    stage: &Path,
    destination: &Path,
) -> io::Result<Installed> {
    let app = bundle::assemble(program, notices, version, stage)?;
    bundle::sign(&app)?;
    swap::install(&app, destination)
}
