//! The differences of the systems in the connection layer. Each one has a variant for unix
//! (macOS, and Linux for CI) and for Windows with the same names, so the code outside this
//! module has no `cfg` of a system.
//!
//! - The environment of a child: the login shell on unix, the process itself on Windows.
//! - The variable of the home folder, and the fixed folders after `PATH`.
//! - A variable name: Windows does not tell `Path` from `PATH`.
//! - A program file: a mode bit on unix, an extension of `PATHEXT` on Windows (`doctl.exe`).
//! - A child and each process that it starts: a session and a process group on unix, a job
//!   object on Windows. [`Tree`] ends them all.
//! - The addresses of the network interfaces.
//! - The time that the computer slept: the monotonic clock of Windows counts a sleep.

#[cfg(unix)]
#[path = "unix/mod.rs"]
mod system;
#[cfg(windows)]
#[path = "windows/mod.rs"]
mod system;

pub use self::system::FIXED_FOLDERS;
pub(crate) use self::system::{
    HOME, HOME_FOLDERS, Started, Tree, environment, interface_addresses, is_bare, is_program, program_files, start,
    time_asleep, variable_key,
};
