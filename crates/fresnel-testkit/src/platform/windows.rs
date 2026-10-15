//! The Windows variant: a program from Explorer gets the full environment of the user, so
//! the environment of the test sets the variables that a profile sets on unix.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use fresnel_net::env::ProcessEnv;

use crate::KitError;

/// A program of a test is a batch file.
pub(crate) const PROGRAM_EXTENSION: &str = ".cmd";
/// The variables of the system that a program needs: Winsock reads the folder of the system,
/// and `cmd.exe` runs a batch file.
const SYSTEM: [&str; 5] = ["SystemRoot", "SystemDrive", "windir", "ComSpec", "PATHEXT"];

/// The environment of a program that starts from Explorer with this home folder: the
/// folders of the app below it, `KUBECONFIG` with these files of `work`, and `plugins` and
/// the folder of the system in `PATH`.
pub(crate) fn desktop_env(folder: &Path, kubeconfig_files: &[&str]) -> Result<ProcessEnv, KitError> {
    let mut env = ProcessEnv::default();
    for name in SYSTEM {
        if let Some(value) = std::env::var_os(name) {
            env.set(name, value);
        }
    }
    env.set_home(folder);
    env.set("APPDATA", folder.join(r"AppData\Roaming"));
    env.set("LOCALAPPDATA", folder.join(r"AppData\Local"));
    let files = kubeconfig_files.iter().map(|file| folder.join("work").join(file));
    env.set("KUBECONFIG", join(files)?);
    let system = PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| OsString::from(r"C:\Windows")));
    env.set("PATH", join([folder.join("plugins"), system.join("System32"), system])?);
    Ok(env)
}

fn join(paths: impl IntoIterator<Item = PathBuf>) -> Result<OsString, KitError> {
    std::env::join_paths(paths).map_err(|error| KitError::because("a folder of the test has a quote", error))
}

/// A batch file runs by its extension.
pub(crate) fn make_program(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

/// Writes a file. The folder of a test is in the temporary folder of the user, which only
/// that user can open.
pub(crate) fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    fs::write(path, text)
}

/// Makes a folder in the temporary folder of the user, which only that user can open.
pub(crate) fn create_private_folder(path: &Path) -> std::io::Result<()> {
    fs::create_dir(path)
}
