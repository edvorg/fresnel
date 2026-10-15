//! A home folder for one test.
//!
//! A program that starts from Finder has only `HOME` and a short `PATH`, and it gets the
//! rest from the login shell of the user; a program from Explorer gets each variable of the
//! user. A test gives the app this folder as its home, with the variables of a profile of the
//! test (`platform`), so the app never reads a file of the user.

use std::path::{Path, PathBuf};

use fresnel_net::env::ProcessEnv;

use crate::{KitError, platform};

/// A home folder with the profile of a test. Its drop removes it.
#[derive(Debug)]
pub struct Home {
    folder: PathBuf,
    env: ProcessEnv,
}

impl Home {
    /// Makes the folder with the folders `plugins`, `state` and `work`. The profile adds
    /// `plugins` to `PATH` and sets `KUBECONFIG` to these files of `work`, as the profile of
    /// a user does.
    pub fn new(label: &str, kubeconfig_files: &[&str]) -> Result<Self, KitError> {
        let folder = std::env::temp_dir().join(format!("fresnel-test-home-{label}-{:08x}", fastrand::u32(..)));
        std::fs::create_dir(&folder)?;
        // The drop removes the folder also after an error below.
        let mut home = Self { folder, env: ProcessEnv::default() };
        for sub in ["plugins", "state", "work"] {
            std::fs::create_dir(home.path(sub))?;
        }
        home.env = platform::desktop_env(&home.folder, kubeconfig_files)?;
        Ok(home)
    }

    /// The folder itself: the home folder of the app.
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// The path of a file of the folder.
    pub fn path(&self, file: &str) -> PathBuf {
        self.folder.join(file)
    }

    /// Writes a file of the folder.
    pub fn write(&self, file: &str, text: &str) -> Result<(), KitError> {
        Ok(std::fs::write(self.path(file), text)?)
    }

    /// The text of a file of the folder. Empty when the file is absent.
    pub fn read(&self, file: &str) -> String {
        std::fs::read_to_string(self.path(file)).unwrap_or_default()
    }

    /// Adds a program to the folder `plugins`: a shell script with this text, or a batch
    /// file `<name>.cmd` on Windows.
    pub fn add_program(&self, name: &str, script: &str) -> Result<(), KitError> {
        let file = format!("plugins/{name}{}", platform::PROGRAM_EXTENSION);
        self.write(&file, script)?;
        Ok(platform::make_program(&self.path(&file))?)
    }

    /// The environment of a program that the user starts from Finder or Explorer with this
    /// home folder.
    pub fn finder_env(&self) -> ProcessEnv {
        self.env.clone()
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.folder) {
            eprintln!("cannot remove {}: {error}", self.folder.display());
        }
    }
}
