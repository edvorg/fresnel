//! Helpers of the unit tests.

use std::path::{Path, PathBuf};

/// A folder for the files of one test. Its drop removes it.
pub(crate) struct Scratch(PathBuf);

impl Scratch {
    pub(crate) fn new(label: &str) -> Self {
        let folder = std::env::temp_dir().join(format!("fresnel-test-{label}-{:08x}", fastrand::u32(..)));
        std::fs::create_dir(&folder).expect("a new folder");
        Self(folder)
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }

    /// The names of the files of the folder, in the sequence of the alphabet.
    pub(crate) fn files(&self) -> Vec<String> {
        let entries = std::fs::read_dir(&self.0).expect("the test folder");
        let mut names: Vec<String> =
            entries.map(|entry| entry.expect("an entry").file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }

    pub(crate) fn read(&self, file: &str) -> String {
        std::fs::read_to_string(self.0.join(file)).expect("a file of the test folder")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0) {
            eprintln!("cannot remove {}: {error}", self.0.display());
        }
    }
}
