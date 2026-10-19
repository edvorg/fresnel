//! `cargo xtask windows`: builds `fresnel.exe` for Windows with its icon and its version, the
//! program of a release, and copies it with its license files into a folder.
//!
//! - `resource`: the resource file with the icon and the version, an input of the linker.

mod resource;

use std::io;
use std::path::{Path, PathBuf};

use crate::notices::Notices;
use crate::program;
use crate::version::Version;

/// The name of the program, and of its resource file in the target folder.
const PROGRAM: &str = "fresnel.exe";
const RESOURCE_FILE: &str = "fresnel.res";

/// Builds the release program of `version` with its resources and copies it into `destination`,
/// with the license files next to it. Returns the path of the copy. `root` is the root of the
/// workspace. The notices come first: a crate with no license text stops it before the build.
pub fn run(root: &Path, destination: &Path, version: &Version) -> io::Result<PathBuf> {
    if !cfg!(windows) {
        return Err(io::Error::new(io::ErrorKind::Unsupported, "the resource file is for the linker of Windows only"));
    }
    let notices = Notices::of_workspace(root)?;
    let target = crate::target_folder(root);
    std::fs::create_dir_all(&target)?;
    let resources = target.join(RESOURCE_FILE);
    std::fs::write(&resources, resource::file(version)?)?;
    let program = program::build(root, version, Some(&resources))?;
    std::fs::create_dir_all(destination)?;
    let copy = destination.join(PROGRAM);
    std::fs::copy(&program, &copy)?;
    notices.write_into(destination)?;
    Ok(copy)
}
