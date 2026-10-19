//! The differences of the systems in the connection layer. Each one has a variant for unix
//! (macOS, and Linux for CI) and for Windows with the same names, so the code outside this
//! module has no `cfg` of a system.
//!
//! - The environment of a child: the login shell on unix, the process itself on Windows.
//! - The variable of the home folder, the folders of `.kube/config` and the fixed folders
//!   after `PATH`.
//! - A variable name: Windows does not tell `Path` from `PATH`.
//! - A program file: a mode bit on unix, an extension of `PATHEXT` on Windows (`doctl.exe`).
//! - A child and each process that it starts: a session and a process group on unix, a job
//!   object on Windows. [`Tree`] ends them all.
//! - The addresses of the network interfaces.
//! - The time that the computer slept: the monotonic clock of Windows counts a sleep.
//! - The bind of a local listener with no shared port, and the bytes that a local client did
//!   not take before a reset (`local`).

#[cfg(unix)]
#[path = "unix/mod.rs"]
mod system;
#[cfg(windows)]
#[path = "windows/mod.rs"]
mod system;

pub use self::system::{FIXED_FOLDERS, LOGIN_SHELL};
pub(crate) use self::system::{
    HOME, HOME_FOLDERS, Started, Tree, environment, interface_addresses, is_bare, is_program, kube_homes, own_port,
    program_files, share_port, start, time_asleep, unacknowledged, variable_key,
};
