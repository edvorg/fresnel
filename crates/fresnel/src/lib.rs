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

use std::process::ExitCode;
use std::sync::Arc;

use fresnel_core::net::env::ProcessEnv;

/// Runs the program to its end.
pub fn run() -> ExitCode {
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

/// Tells why the program stops at its start: on stderr, and in a dialog of the system where
/// no console shows it (`platform`).
fn tell_start_error(text: &str) {
    eprintln!("fresnel: {text}");
    platform::show_start_error(text);
}
