//! The desktop program: the window, the tokio runtime and the log setup.
//!
//! - [`boot`]: the start before the window, and the quit sequence.
//! - [`frame`]: one frame of the window, also for a test with no window.
//! - [`folders`]: where the files are. [`lock`]: one process of the app at a time.
//! - [`log_file`]: the journal on the disk. [`panic_report`]: a file for each panic.
//! - [`nap`]: the macOS activity against App Nap. `window`: the window itself.

pub mod boot;
pub mod folders;
pub mod frame;
pub mod lock;
pub mod log_file;
pub mod nap;
pub mod panic_report;
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
            eprintln!("fresnel: {error}");
            return ExitCode::FAILURE;
        }
    };
    let window = window::run(&boot);
    if let Err(error) = &window {
        boot.journal().error("app", format!("the window did not open: {error}"));
        eprintln!("fresnel: the window did not open: {error}");
    }
    boot.quit();
    if window.is_ok() { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}
