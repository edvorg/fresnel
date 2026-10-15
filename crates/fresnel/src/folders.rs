//! The folders of the app on the disk (plan section 12.5), from the environment of the
//! process: the library of the home folder on macOS, the application data on Windows.
//!
//! A debug build has its own folders. Thus a build of a developer never reads or changes
//! the files of the installed app, and the two can run at the same time.

use std::io;
use std::path::PathBuf;

use fresnel_core::net::env::ProcessEnv;

use crate::platform;

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
    /// The files that the user saves, such as a log: the folder of downloads of the user,
    /// which both builds share. The first save makes it.
    pub downloads: PathBuf,
}

impl Folders {
    /// The folders of a build for the user of an environment. `None` when a variable that
    /// names them is absent (`platform::FOLDER_VARIABLES`).
    pub fn of(env: &ProcessEnv, build: Build) -> Option<Self> {
        let downloads = PathBuf::from(env.home()?).join("Downloads");
        let (data, logs) = platform::folders(env, build.folder_name())?;
        Some(Self { data, logs, downloads })
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
        let home = std::env::temp_dir().join("tester");
        let mut env = ProcessEnv::default();
        env.set_home(&home);
        env.set("APPDATA", home.join("Roaming"));
        env.set("LOCALAPPDATA", home.join("Local"));
        let of = |build| Folders::of(&env, build).expect("the folders");
        let (release, debug) = (of(Build::Release), of(Build::Debug));
        if cfg!(target_os = "macos") {
            assert_eq!(release.data, home.join("Library/Application Support/fresnel"));
            assert_eq!(release.logs, home.join("Library/Logs/fresnel"));
            assert_eq!(debug.data, home.join("Library/Application Support/fresnel-debug"));
            assert_eq!(debug.logs, home.join("Library/Logs/fresnel-debug"));
        }
        if cfg!(windows) {
            assert_eq!(
                (&release.data, &release.logs),
                (&home.join(r"Roaming\fresnel"), &home.join(r"Local\fresnel\logs"))
            );
            assert_eq!(debug.data, home.join(r"Roaming\fresnel-debug"));
        }
        assert_eq!((&release.downloads, &debug.downloads), (&home.join("Downloads"), &home.join("Downloads")));
        for (one, other) in [(&release.data, &debug.data), (&release.logs, &debug.logs)] {
            assert!(one.starts_with(&home) && other.starts_with(&home));
            assert!(!one.starts_with(other) && !other.starts_with(one), "{one:?} and {other:?}");
        }
        assert_eq!(Build::current(), Build::Debug, "a test is a debug build");
        assert_eq!(Folders::of(&ProcessEnv::default(), Build::Debug), None, "no variable names the folders");
    }
}
