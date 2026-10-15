//! The unix variant: a program from Finder gets `HOME` and a short `PATH`, and the login
//! shell gives the rest.

use std::fs;
use std::io::Write as _;
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::Path;

use fresnel_net::env::ProcessEnv;

use crate::KitError;

/// The `PATH` of a program that starts from Finder.
pub const FINDER_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";
/// A shell script has no extension.
pub(crate) const PROGRAM_EXTENSION: &str = "";

/// Writes the shell profile of the home folder, which adds `plugins` to `PATH` and sets
/// `KUBECONFIG` to these files of `work`, as the profile of a user does. A shell reads the
/// profile file of its own type. Gives the environment of a program from Finder.
pub(crate) fn desktop_env(folder: &Path, kubeconfig_files: &[&str]) -> Result<ProcessEnv, KitError> {
    let files: Vec<String> = kubeconfig_files.iter().map(|file| format!("$HOME/work/{file}")).collect();
    let profile = format!("export PATH=\"$HOME/plugins:$PATH\"\nexport KUBECONFIG=\"{}\"\n", files.join(":"));
    for file in [".zprofile", ".bash_profile", ".profile"] {
        fs::write(folder.join(file), &profile)?;
    }
    // With no file for interactive shells, zsh asks a new user some questions.
    fs::write(folder.join(".zshrc"), "")?;
    let mut env = ProcessEnv::default();
    env.set_home(folder);
    env.set("PATH", FINDER_PATH);
    Ok(env)
}

/// Lets each user run the file.
pub(crate) fn make_program(path: &Path) -> std::io::Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))
}

/// Writes a file that only its owner can read.
pub(crate) fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    let mut file = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
    file.write_all(text.as_bytes())
}

/// Makes a folder that only its owner can open.
pub(crate) fn create_private_folder(path: &Path) -> std::io::Result<()> {
    fs::DirBuilder::new().mode(0o700).create(path)
}
