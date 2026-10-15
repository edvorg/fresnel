//! Reads the environment of the login shell, one time, with a time limit.
//!
//! A program that starts from Finder has `PATH=/usr/bin:/bin:/usr/sbin:/sbin`. The credential
//! plugin is in a folder that only the shell profile adds.

use std::ffi::{OsStr, OsString};
use std::ops::Range;
use std::os::unix::ffi::OsStrExt as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use nix::unistd::{Uid, User};

use crate::env::shell::MAX_DUMP_BYTES;
use crate::env::{EnvSettings, EnvSource, ProcessEnv, ShellFailure, ShellProblem};
use crate::process::{self, End, Program, RunLimits, tail};

/// The shell of macOS, for a user with no shell in the user database and no `SHELL`.
const FALLBACK_SHELL: &str = "/bin/zsh";
const DETAIL_CHARACTERS: usize = 400;
/// The variables that the shell gets from this module and not from the user.
const INJECTED: [(&str, &str); 2] = [("DISABLE_AUTO_UPDATE", "true"), ("TERM", "dumb")];

/// The environment of the login shell, or else the environment of the process.
pub(crate) async fn environment(base: &ProcessEnv, settings: &EnvSettings) -> (ProcessEnv, EnvSource) {
    let shell = settings.shell.clone().unwrap_or_else(|| login_shell(base));
    match dump(&shell, base, settings.shell_limit).await {
        Ok(vars) => (vars, EnvSource::LoginShell(shell)),
        Err(problem) => (base.clone(), EnvSource::Fallback(problem)),
    }
}

/// The shell of the user database. `SHELL` can be absent for a program from Finder.
fn login_shell(base: &ProcessEnv) -> PathBuf {
    let from_database = User::from_uid(Uid::current()).ok().flatten().map(|user| user.shell);
    from_database
        .filter(|shell| !shell.as_os_str().is_empty())
        .or_else(|| base.get("SHELL").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from(FALLBACK_SHELL))
}

/// Runs the shell as a login shell and reads the environment that a terminal would have.
/// The shell starts from `base`, so its profile decides which values of the process stay.
///
/// The run ends when the dump is complete. It does not wait for the end of the shell: a
/// profile can run a slow logout step or leave a process, and the kill of the group ends both.
async fn dump(shell: &Path, base: &ProcessEnv, limit: Duration) -> Result<ProcessEnv, ShellProblem> {
    let mark = unique_mark();
    // The marks remove text that a profile prints. `env -0` keeps a value with a newline.
    let script = format!("printf %s {mark}; /usr/bin/env -0; printf %s {mark}");
    let mut env = base.clone();
    INJECTED.into_iter().for_each(|(name, value)| env.set(name, value));
    // `-i` is necessary: many users set PATH in the file for interactive shells.
    let args = ["-l", "-i", "-c", &script].map(OsString::from).into();
    let program = Program { path: shell.to_owned(), args, env };
    let problem = |failure, detail| ShellProblem { shell: shell.to_owned(), failure, detail };

    let limits = RunLimits { time: limit, output_bytes: MAX_DUMP_BYTES };
    let output = process::run(&program, limits, |out| dump_range(out, mark.as_bytes()).is_some())
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
}
