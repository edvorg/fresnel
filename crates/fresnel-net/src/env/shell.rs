//! Reads the environment of the login shell, one time, with a time limit.
//!
//! A program that starts from Finder has `PATH=/usr/bin:/bin:/usr/sbin:/sbin`. The credential
//! plugin is in a folder that only the shell profile adds.

use std::ffi::OsStr;
use std::fmt::{self, Display};
use std::ops::Range;
use std::os::unix::ffi::OsStrExt as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::process::Command;

use super::vars::ProcessEnv;
use crate::process::{self, End, RunLimits, tail};

/// An environment is some kilobytes. A profile that prints more is broken.
const MAX_DUMP_BYTES: usize = 4 * 1024 * 1024;
const DETAIL_CHARACTERS: usize = 400;
/// The variables that the shell gets from this module and not from the user.
const INJECTED: [(&str, &str); 2] = [("DISABLE_AUTO_UPDATE", "true"), ("TERM", "dumb")];

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
    shell: PathBuf,
    failure: ShellFailure,
    detail: String,
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

/// Runs the shell as a login shell and reads the environment that a terminal would have.
/// The shell starts from `base`, so its profile decides which values of the process stay.
///
/// The run ends when the dump is complete. It does not wait for the end of the shell: a
/// profile can run a slow logout step or leave a process, and the kill of the group ends both.
pub(super) async fn dump(shell: &Path, base: &ProcessEnv, limit: Duration) -> Result<ProcessEnv, ShellProblem> {
    let mark = unique_mark();
    // The marks remove text that a profile prints. `env -0` keeps a value with a newline.
    let script = format!("printf %s {mark}; /usr/bin/env -0; printf %s {mark}");
    let mut command = Command::new(shell);
    // `-i` is necessary: many users set PATH in the file for interactive shells.
    command.args(["-l", "-i", "-c", &script]).env_clear().envs(base.iter()).envs(INJECTED);
    let problem = |failure, detail| ShellProblem { shell: shell.to_owned(), failure, detail };

    let limits = RunLimits { time: limit, output_bytes: MAX_DUMP_BYTES };
    let output = process::run(command, limits, |out| dump_range(out, mark.as_bytes()).is_some())
        .await
        .map_err(|error| problem(ShellFailure::Start(error.to_string()), String::new()))?;
    let failure = match output.end {
        End::Complete => match parse(&output.stdout, mark.as_bytes(), base) {
            Some(vars) => return Ok(vars),
            None => ShellFailure::NoDump("the dump is not complete".to_owned()),
        },
        End::Exited(status) => ShellFailure::NoDump(status.to_string()),
        End::TimedOut => ShellFailure::Timeout(limit),
        End::TooMuchOutput => ShellFailure::TooMuchOutput,
        End::Failed(error) => ShellFailure::Read(error.to_string()),
    };
    Err(problem(failure, tail(&output.stderr, DETAIL_CHARACTERS)))
}

fn unique_mark() -> String {
    format!("__fresnel_env_{}_{:08x}__", std::process::id(), fastrand::u32(..))
}

/// The byte range between the first mark and the last mark.
fn dump_range(out: &[u8], mark: &[u8]) -> Option<Range<usize>> {
    let first = out.windows(mark.len()).position(|window| window == mark)?;
    let last = out.windows(mark.len()).rposition(|window| window == mark)?;
    (last > first).then(|| first + mark.len()..last)
}

fn parse(out: &[u8], mark: &[u8], base: &ProcessEnv) -> Option<ProcessEnv> {
    let dump = out.get(dump_range(out, mark)?)?;
    let pairs = dump.split(|byte| *byte == 0).filter_map(|pair| {
        let equals = pair.iter().position(|byte| *byte == b'=').filter(|position| *position > 0)?;
        let (name, value) = pair.split_at(equals);
        Some((OsStr::from_bytes(name), OsStr::from_bytes(value.get(1..)?)))
    });
    let mut vars: ProcessEnv = pairs.collect();
    // The shell got these values from this module. The value of the process comes back.
    for (name, _) in INJECTED {
        match base.get(name) {
            Some(value) => vars.set(name, value),
            None => vars.remove(name),
        }
    }
    Some(vars)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none() -> ProcessEnv {
        ProcessEnv::default()
    }

    #[test]
    fn the_parse_takes_only_the_text_between_the_marks() {
        let out = b"profile noise\nMARKA=1\0PATH=/x:/y\0MULTI=line1\nline2\0EMPTY=\0=bad\0MARKlogout noise";
        let vars = parse(out, b"MARK", &none()).expect("a dump");
        assert_eq!(vars.len(), 4);
        assert_eq!(vars.path_entries().len(), 2);
        assert_eq!(vars.get("MULTI").and_then(OsStr::to_str), Some("line1\nline2"));
        assert_eq!(vars.get("EMPTY"), Some(OsStr::new("")));
    }

    #[test]
    fn one_mark_is_not_a_dump() {
        assert!(dump_range(b"MARKA=1\0", b"MARK").is_none());
        assert!(dump_range(b"", b"MARK").is_none());
        assert_eq!(dump_range(b"MARKMARK", b"MARK"), Some(4..4));
    }

    #[test]
    fn an_injected_variable_gets_the_value_of_the_process_again() {
        let out = b"MARKTERM=dumb\0DISABLE_AUTO_UPDATE=true\0A=1\0MARK";
        assert_eq!(parse(out, b"MARK", &none()).expect("a dump").len(), 1);
        let base: ProcessEnv = [("TERM", "xterm")].into_iter().collect();
        let vars = parse(out, b"MARK", &base).expect("a dump");
        assert_eq!((vars.len(), vars.get("TERM")), (2, Some(OsStr::new("xterm"))));
    }

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
