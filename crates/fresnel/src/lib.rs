//! The desktop program: the window, the tokio runtime and the log setup.
//!
//! - [`boot`]: the start before the window, and the quit sequence.
//! - [`frame`]: one frame of the window, also for a test with no window.
//! - [`folders`]: where the files are. [`lock`]: one process of the app at a time.
//! - [`log_file`]: the journal on the disk. [`panic_report`]: a file for each panic.
//! - [`nap`]: the macOS activity against App Nap. [`notify`]: the notifications of the system.
//! - `platform`: each difference of macOS, Windows and Linux. `window`: the window itself.

pub mod boot;
pub mod folders;
pub mod frame;
pub mod lock;
pub mod log_file;
pub mod nap;
pub mod notify;
pub mod panic_report;
mod platform;
#[cfg(test)]
mod testing;
mod window;

use std::ffi::OsString;
use std::io::Write as _;
use std::process::ExitCode;
use std::sync::Arc;

use fresnel_core::net::env::ProcessEnv;

/// The version of the program. A build of `cargo xtask` gives the text of its version, with the
/// run number of the release workflow (`0.1.0+build.7`); a plain build has the workspace version.
pub const VERSION: &str = match option_env!("FRESNEL_VERSION") {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};

/// Runs the program to its end.
pub fn run() -> ExitCode {
    if asks_version(std::env::args_os().skip(1)) {
        return tell_version();
    }
    let boot = match boot::Boot::start(ProcessEnv::from_process()) {
        Ok(boot) => Arc::new(boot),
        Err(error) => {
            tell_start_error(&error.to_string());
            return ExitCode::FAILURE;
        }
    };
    let window = window::run(&boot);
    if let Err(error) = &window {
        let text = format!("the window did not open: {error}");
        boot.journal().error("app", &text);
        tell_start_error(&text);
    }
    boot.quit();
    if window.is_ok() { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

/// `--version` as the first argument: the program prints its version and ends before its lock,
/// its folders and its window. The release workflow checks the program of each release with it.
fn asks_version(mut arguments: impl Iterator<Item = OsString>) -> bool {
    arguments.next().is_some_and(|first| first == "--version")
}

/// Prints the version. A release build for Windows has no console: a pipe or a file of the
/// process that starts it still gets the text, and with neither the text goes nowhere.
fn tell_version() -> ExitCode {
    match writeln!(std::io::stdout(), "fresnel {VERSION}") {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("fresnel: cannot print the version: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Tells why the program stops at its start: on stderr, and in a dialog of the system where
/// no console shows it (`platform`).
fn tell_start_error(text: &str) {
    eprintln!("fresnel: {text}");
    platform::show_start_error(text);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(texts: &[&str]) -> impl Iterator<Item = OsString> {
        texts.iter().map(OsString::from).collect::<Vec<_>>().into_iter()
    }

    #[test]
    fn only_version_as_the_first_argument_asks_for_the_version() {
        assert!(asks_version(arguments(&["--version"])));
        assert!(asks_version(arguments(&["--version", "more"])));
        for other in [&[][..], &["-version"], &["--versions"], &["-psn_0_12345"], &["more", "--version"]] {
            assert!(!asks_version(arguments(other)), "{other:?}");
        }
    }
}
