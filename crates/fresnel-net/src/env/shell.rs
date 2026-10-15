//! Why the login shell gave no environment. Only the unix variant of `platform` runs a
//! login shell; the type is a part of [`super::EnvSource`] on each system.

use std::fmt::{self, Display};
use std::path::PathBuf;
use std::time::Duration;

/// An environment is some kilobytes. A profile that prints more is broken.
pub(crate) const MAX_DUMP_BYTES: usize = 4 * 1024 * 1024;

/// Why the login shell gave no environment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShellFailure {
    /// The shell did not start.
    Start(String),
    /// The dump was not complete in the time limit.
    Timeout(Duration),
    /// The shell ended with this status and wrote no dump, for example a profile that ends
    /// with `exec fish`.
    NoDump(String),
    /// The shell wrote more than the output limit.
    TooMuchOutput,
    /// The output of the shell could not be read.
    Read(String),
}

/// The login shell gave no environment. The app then uses the fixed folders.
///
/// `Display` has no text that the profile wrote, so it is safe for a log.
#[derive(Clone, PartialEq, Eq)]
pub struct ShellProblem {
    pub(crate) shell: PathBuf,
    pub(crate) failure: ShellFailure,
    pub(crate) detail: String,
}

impl ShellProblem {
    /// Why the shell gave no environment.
    pub fn failure(&self) -> &ShellFailure {
        &self.failure
    }

    /// The last text that the profile wrote to stderr. Show it in the UI only: a profile can
    /// print a secret.
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl Display for ShellProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let shell = self.shell.display();
        match &self.failure {
            ShellFailure::Start(reason) => write!(f, "cannot start the login shell {shell}: {reason}"),
            ShellFailure::Timeout(limit) => write!(f, "the login shell {shell} gave no environment in {limit:?}"),
            ShellFailure::NoDump(status) => write!(f, "the login shell {shell} gave no environment ({status})"),
            ShellFailure::TooMuchOutput => write!(f, "the login shell {shell} wrote more than {MAX_DUMP_BYTES} bytes"),
            ShellFailure::Read(reason) => write!(f, "cannot read the output of the login shell {shell}: {reason}"),
        }
    }
}

impl fmt::Debug for ShellProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ShellProblem({self}; {} characters of detail)", self.detail.chars().count())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_text_of_a_problem_has_no_profile_output() {
        let problem = ShellProblem {
            shell: PathBuf::from("/bin/zsh"),
            failure: ShellFailure::Timeout(Duration::from_secs(5)),
            detail: "export TOKEN=secret-of-the-profile".to_owned(),
        };
        assert_eq!(problem.to_string(), "the login shell /bin/zsh gave no environment in 5s");
        assert!(!format!("{problem:?}").contains("secret"), "{problem:?}");
        assert!(problem.detail().contains("secret-of-the-profile"));
    }
}
