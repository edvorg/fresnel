//! The release build of the program `fresnel`, for the bundle of macOS and for Windows.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

use crate::version::{VERSION_VARIABLE, Version};

const PACKAGE: &str = "fresnel";

/// Builds the release program with the cargo that runs this task and gives it the text of
/// `version`. `linker_input` is one more file for the linker of the program alone, such as the
/// resource file of Windows; `cargo rustc` gives it to no other crate. Returns the program.
pub(crate) fn build(root: &Path, version: &Version, linker_input: Option<&Path>) -> io::Result<PathBuf> {
    let mut command = crate::cargo(root);
    command.args(["rustc", "--release", "--locked", "--package", PACKAGE, "--bin", PACKAGE]);
    if let Some(file) = linker_input {
        let mut argument = OsString::from("link-arg=");
        argument.push(file);
        command.args(["--", "-C"]).arg(argument);
    }
    let status = command.env(VERSION_VARIABLE, version.to_string()).status()?;
    if !status.success() {
        return Err(io::Error::other(format!("the release build failed ({status})")));
    }
    Ok(crate::target_folder(root).join("release").join(format!("{PACKAGE}{}", std::env::consts::EXE_SUFFIX)))
}
