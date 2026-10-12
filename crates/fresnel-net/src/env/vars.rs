//! The environment that the app gives to its child processes.
//!
//! The app never changes its own process environment: `std::env::set_var` is not safe in a
//! program with threads. Each child gets this value in place of the process environment.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

const PATH: &str = "PATH";

/// A set of environment variables.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct ProcessEnv {
    vars: BTreeMap<OsString, OsString>,
}

impl ProcessEnv {
    /// A copy of the environment of this process.
    pub fn from_process() -> Self {
        std::env::vars_os().collect()
    }

    /// The value of one variable.
    pub fn get(&self, name: &str) -> Option<&OsStr> {
        self.vars.get(OsStr::new(name)).map(OsString::as_os_str)
    }

    /// Sets one variable.
    pub fn set(&mut self, name: impl Into<OsString>, value: impl Into<OsString>) {
        self.vars.insert(name.into(), value.into());
    }

    /// Removes one variable.
    pub fn remove(&mut self, name: &str) {
        self.vars.remove(OsStr::new(name));
    }

    /// Each variable with its value.
    pub fn iter(&self) -> impl Iterator<Item = (&OsStr, &OsStr)> {
        self.vars.iter().map(|(name, value)| (name.as_os_str(), value.as_os_str()))
    }

    /// The count of variables.
    pub fn len(&self) -> usize {
        self.vars.len()
    }

    /// True with no variable.
    pub fn is_empty(&self) -> bool {
        self.vars.is_empty()
    }

    /// The folders of `PATH`, in their sequence.
    pub fn path_entries(&self) -> Vec<PathBuf> {
        self.get(PATH).map(|path| std::env::split_paths(path).collect()).unwrap_or_default()
    }

    /// Adds folders to `PATH`, `first` before the present entries and `last` after them. A
    /// folder that `PATH` has already keeps its place.
    pub(crate) fn extend_path(&mut self, first: &[PathBuf], last: &[PathBuf]) {
        let mut entries = Vec::new();
        for entry in first.iter().cloned().chain(self.path_entries()).chain(last.iter().cloned()) {
            if !entries.contains(&entry) {
                entries.push(entry);
            }
        }
        // `join_paths` refuses a folder with a colon in its name. `PATH` cannot hold such a folder.
        entries.retain(|entry| !entry.as_os_str().as_encoded_bytes().contains(&b':'));
        if let Ok(path) = std::env::join_paths(entries) {
            self.set(PATH, path);
        }
    }

    /// Finds a program as `kubectl` does for a credential plugin. A bare name goes through
    /// `PATH`. A command with a slash is a path, and a relative path starts from `folder`:
    /// the folder of the kubeconfig file.
    pub fn find_program(&self, command: &str, folder: Option<&Path>) -> Option<PathBuf> {
        if !command.contains('/') {
            return self.path_entries().into_iter().map(|entry| entry.join(command)).find(|path| is_executable(path));
        }
        let path = match folder {
            Some(folder) if Path::new(command).is_relative() => folder.join(command),
            Some(_) | None => PathBuf::from(command),
        };
        is_executable(&path).then_some(path)
    }
}

impl<K: Into<OsString>, V: Into<OsString>> FromIterator<(K, V)> for ProcessEnv {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(pairs: I) -> Self {
        Self { vars: pairs.into_iter().map(|(name, value)| (name.into(), value.into())).collect() }
    }
}

/// Shows counts only. A value can be a token of some other tool.
impl fmt::Debug for ProcessEnv {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProcessEnv")
            .field("variables", &self.len())
            .field("path_entries", &self.path_entries().len())
            .finish()
    }
}

fn is_executable(path: &Path) -> bool {
    path.metadata().is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> ProcessEnv {
        pairs.iter().copied().collect()
    }

    fn folders(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn new_folders_go_before_and_after_and_a_present_folder_keeps_its_place() {
        let mut vars = env(&[("PATH", "/usr/bin:/bin")]);
        vars.extend_path(&folders(&["/setting", "/bin"]), &folders(&["/opt/homebrew/bin", "/usr/bin", "/bad:name"]));
        assert_eq!(vars.get("PATH"), Some(OsStr::new("/setting:/bin:/usr/bin:/opt/homebrew/bin")));

        let mut empty = env(&[]);
        empty.extend_path(&[], &folders(&["/usr/local/bin"]));
        assert_eq!(empty.path_entries(), folders(&["/usr/local/bin"]));
    }

    #[test]
    fn the_debug_text_has_no_value() {
        let text = format!("{:?}", env(&[("TOKEN", "very-secret"), ("PATH", "/usr/bin")]));
        assert_eq!(text, "ProcessEnv { variables: 2, path_entries: 1 }");
    }

    #[test]
    fn a_bare_name_goes_through_path_and_a_relative_path_starts_from_the_folder() {
        let vars = env(&[("PATH", "/nonexistent:/bin")]);
        assert_eq!(vars.find_program("sh", None), Some(PathBuf::from("/bin/sh")));
        assert_eq!(env(&[("PATH", "/nonexistent")]).find_program("sh", None), None);
        assert_eq!(env(&[]).find_program("/bin/sh", Some(Path::new("/tmp"))), Some(PathBuf::from("/bin/sh")));
        assert_eq!(env(&[]).find_program("./sh", Some(Path::new("/bin"))), Some(PathBuf::from("/bin/./sh")));
        assert_eq!(env(&[]).find_program("bin/no-such-program", Some(Path::new("/"))), None);
    }
}
