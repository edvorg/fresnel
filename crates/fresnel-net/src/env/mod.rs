//! The environment for the child processes of the app (plan section 8.1).
//!
//! On macOS, the app reads the environment of the login shell one time at its start. If that
//! fails, the app continues with its own environment. On Windows, the process has the full
//! environment of the user, and no shell runs. In each case `PATH` also gets the folders of
//! the settings and some fixed folders, so that a credential plugin is found.

pub(crate) mod shell;
mod vars;

use std::path::PathBuf;
use std::time::Duration;

use tokio::time::Instant;

pub use crate::platform::FIXED_FOLDERS;
use crate::platform::{self, HOME_FOLDERS};
pub use shell::{ShellFailure, ShellProblem};
pub use vars::ProcessEnv;

/// The settings of the environment resolve.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvSettings {
    /// The time limit for the login shell. Measured: 11 to 67 ms on the machine of the user.
    pub shell_limit: Duration,
    /// Folders that go before each other entry of `PATH`.
    pub extra_folders: Vec<PathBuf>,
    /// Folders that go after each other entry of `PATH`: a plugin of a package manager is
    /// there, also when the login shell gave no environment.
    pub fixed_folders: Vec<PathBuf>,
    /// The shell to run. `None` selects the login shell of the user.
    pub shell: Option<PathBuf>,
}

impl Default for EnvSettings {
    fn default() -> Self {
        let fixed_folders = FIXED_FOLDERS.iter().map(PathBuf::from).collect();
        Self { shell_limit: Duration::from_secs(5), extra_folders: Vec::new(), fixed_folders, shell: None }
    }
}

/// Where the variables of an [`Environment`] come from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EnvSource {
    /// The login shell gave them.
    LoginShell(PathBuf),
    /// The login shell gave nothing. The variables are those of the process. The journal
    /// shows the problem.
    Fallback(ShellProblem),
    /// The system has no login shell for the app (Windows): the variables of the process
    /// are complete.
    Process,
}

/// The result of [`resolve`].
#[derive(Clone, Debug)]
pub struct Environment {
    /// The variables for each child process.
    pub vars: ProcessEnv,
    /// Where the variables come from.
    pub source: EnvSource,
    /// The time that the resolve took.
    pub elapsed: Duration,
}

/// Reads the environment of the login shell. `base` is the environment of the process.
///
/// The function always gives an environment: the app must start with a broken profile too.
pub async fn resolve(base: &ProcessEnv, settings: &EnvSettings) -> Environment {
    let started = Instant::now();
    let (mut vars, source) = platform::environment(base, settings).await;
    let mut fixed = settings.fixed_folders.clone();
    if let Some(home) = vars.home().map(PathBuf::from) {
        fixed.extend(HOME_FOLDERS.iter().map(|folder| home.join(folder)));
    }
    vars.extend_path(&settings.extra_folders, &fixed);
    Environment { vars, source, elapsed: started.elapsed() }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An empty home variable names no folder. A home folder after `PATH` would then be a
    /// path below the current folder of the process, where a bare plugin name finds a file.
    #[tokio::test]
    async fn an_empty_home_variable_adds_no_folder_to_path() {
        let mut base: ProcessEnv =
            [("PATH", std::env::join_paths(["/usr/bin"]).expect("a folder"))].into_iter().collect();
        base.set_home("");
        let shell = Some(PathBuf::from("/nonexistent/shell"));
        let settings = EnvSettings { shell, fixed_folders: Vec::new(), ..EnvSettings::default() };
        let resolved = resolve(&base, &settings).await;
        assert_eq!(resolved.vars.path_entries(), [PathBuf::from("/usr/bin")]);
    }
}
