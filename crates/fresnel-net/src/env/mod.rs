//! The environment for the child processes of the app (plan section 8.1).
//!
//! The app reads the environment of the login shell one time at its start. If that fails,
//! the app continues with its own environment. In each case `PATH` also gets the folders of
//! the settings and some fixed folders, so that a credential plugin is found.

mod shell;
mod vars;

use std::path::PathBuf;
use std::time::Duration;

use nix::unistd::{Uid, User};
use tokio::time::Instant;

pub use shell::{ShellFailure, ShellProblem};
pub use vars::ProcessEnv;

/// The shell of macOS, for a user with no shell in the user database and no `SHELL`.
const FALLBACK_SHELL: &str = "/bin/zsh";
/// Where a credential plugin most often is: the default of the fixed folders.
const FIXED_FOLDERS: [&str; 2] = ["/opt/homebrew/bin", "/usr/local/bin"];
/// A fixed folder below the home folder, after the others.
const HOME_FOLDER: &str = ".local/bin";

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
    let shell = settings.shell.clone().unwrap_or_else(|| login_shell(base));
    let (mut vars, source) = match shell::dump(&shell, base, settings.shell_limit).await {
        Ok(vars) => (vars, EnvSource::LoginShell(shell)),
        Err(problem) => (base.clone(), EnvSource::Fallback(problem)),
    };
    let mut fixed = settings.fixed_folders.clone();
    fixed.extend(vars.get("HOME").map(|home| PathBuf::from(home).join(HOME_FOLDER)));
    vars.extend_path(&settings.extra_folders, &fixed);
    Environment { vars, source, elapsed: started.elapsed() }
}

/// The shell of the user database. `SHELL` can be absent for a program from Finder.
fn login_shell(base: &ProcessEnv) -> PathBuf {
    let from_database = User::from_uid(Uid::current()).ok().flatten().map(|user| user.shell);
    from_database
        .filter(|shell| !shell.as_os_str().is_empty())
        .or_else(|| base.get("SHELL").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from(FALLBACK_SHELL))
}
