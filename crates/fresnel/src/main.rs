//! The desktop program. The library of this package holds each part.

// A release build for Windows is a program of the window system: it opens no console.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::process::ExitCode;

fn main() -> ExitCode {
    fresnel::run()
}
