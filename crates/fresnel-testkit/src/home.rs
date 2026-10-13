//! A home folder for one test, with a shell profile.
//!
//! A program that starts from Finder has only `HOME` and a short `PATH`. It gets the rest
//! from the login shell of the user. A test gives the app this folder as `HOME`, so the
//! shell runs the profile of the test and never the profile of the user.

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use fresnel_net::env::ProcessEnv;

use crate::KitError;

/// The `PATH` of a program that starts from Finder.
pub const FINDER_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

/// A home folder with a shell profile. Its drop removes it.
#[derive(Debug)]
pub struct Home(PathBuf);

impl Home {
    /// Makes the folder with the folders `plugins`, `state` and `work`. The profile adds
    /// `plugins` to `PATH` and sets `KUBECONFIG` to these files of `work`, as the profile of
    /// a user does. A shell reads the profile file of its own type.
    pub fn new(label: &str, kubeconfig_files: &[&str]) -> Result<Self, KitError> {
        let folder = std::env::temp_dir().join(format!("fresnel-test-home-{label}-{:08x}", fastrand::u32(..)));
        std::fs::create_dir(&folder)?;
        let home = Self(folder);
        for sub in ["plugins", "state", "work"] {
            std::fs::create_dir(home.path(sub))?;
        }
        let files: Vec<String> = kubeconfig_files.iter().map(|file| format!("$HOME/work/{file}")).collect();
        let profile = format!("export PATH=\"$HOME/plugins:$PATH\"\nexport KUBECONFIG=\"{}\"\n", files.join(":"));
        for file in [".zprofile", ".bash_profile", ".profile"] {
            home.write(file, &profile)?;
        }
        // With no file for interactive shells, zsh asks a new user some questions.
        home.write(".zshrc", "")?;
        Ok(home)
    }

    /// The folder itself: the value of `HOME`.
    pub fn folder(&self) -> &Path {
        &self.0
    }

    /// The path of a file of the folder.
    pub fn path(&self, file: &str) -> PathBuf {
        self.0.join(file)
    }

    /// Writes a file of the folder.
    pub fn write(&self, file: &str, text: &str) -> Result<(), KitError> {
        Ok(std::fs::write(self.path(file), text)?)
    }

    /// The text of a file of the folder. Empty when the file is absent.
    pub fn read(&self, file: &str) -> String {
        std::fs::read_to_string(self.path(file)).unwrap_or_default()
    }

    /// Adds a program to the folder `plugins`: a shell script with this text.
    pub fn add_program(&self, name: &str, script: &str) -> Result<(), KitError> {
        let file = format!("plugins/{name}");
        self.write(&file, script)?;
        Ok(std::fs::set_permissions(self.path(&file), std::fs::Permissions::from_mode(0o755))?)
    }

    /// The environment of a program that starts from Finder with this home folder.
    pub fn finder_env(&self) -> ProcessEnv {
        let mut env = ProcessEnv::default();
        env.set("HOME", self.0.as_os_str());
        env.set("PATH", FINDER_PATH);
        env
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0) {
            eprintln!("cannot remove {}: {error}", self.0.display());
        }
    }
}
