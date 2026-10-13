//! The folders of the app on the disk (plan section 12.5).
//!
//! A debug build has its own folders. Thus a build of a developer never reads or changes
//! the files of the installed app, and the two can run at the same time.

use std::io;
use std::path::{Path, PathBuf};

/// The name of the folders of the installed app.
const RELEASE_NAME: &str = "fresnel";
/// The name of the folders of a debug build.
const DEBUG_NAME: &str = "fresnel-debug";

/// The type of the build that runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Build {
    /// The installed app.
    Release,
    /// A build of a developer.
    Debug,
}

impl Build {
    /// The type of this build.
    pub const fn current() -> Self {
        if cfg!(debug_assertions) { Self::Debug } else { Self::Release }
    }

    const fn folder_name(self) -> &'static str {
        match self {
            Self::Release => RELEASE_NAME,
            Self::Debug => DEBUG_NAME,
        }
    }
}

/// The folders of the app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Folders {
    /// The settings, the forward rules, the layout state and the lock of the process.
    pub data: PathBuf,
    /// The log file and the panic reports.
    pub logs: PathBuf,
}

impl Folders {
    /// The folders of a build below the home folder of the user.
    pub fn of(home: &Path, build: Build) -> Self {
        let name = build.folder_name();
        if cfg!(target_os = "macos") {
            let library = home.join("Library");
            Self { data: library.join("Application Support").join(name), logs: library.join("Logs").join(name) }
        } else {
            let state = home.join(".local/state").join(name);
            Self { data: home.join(".config").join(name), logs: state.join("logs") }
        }
    }

    /// Makes the folders. A folder that exists stays as it is.
    pub fn create(&self) -> io::Result<()> {
        std::fs::create_dir_all(&self.data)?;
        std::fs::create_dir_all(&self.logs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_debug_build_has_its_own_folders() {
        let home = Path::new("/Users/tester");
        let (release, debug) = (Folders::of(home, Build::Release), Folders::of(home, Build::Debug));
        if cfg!(target_os = "macos") {
            assert_eq!(release.data, home.join("Library/Application Support/fresnel"));
            assert_eq!(release.logs, home.join("Library/Logs/fresnel"));
            assert_eq!(debug.data, home.join("Library/Application Support/fresnel-debug"));
            assert_eq!(debug.logs, home.join("Library/Logs/fresnel-debug"));
        }
        for (one, other) in [(&release.data, &debug.data), (&release.logs, &debug.logs)] {
            assert!(one.starts_with(home) && other.starts_with(home));
            assert!(!one.starts_with(other) && !other.starts_with(one), "{one:?} and {other:?}");
        }
        assert_eq!(Build::current(), Build::Debug, "a test is a debug build");
    }
}
