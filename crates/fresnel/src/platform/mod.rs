//! The differences of the systems in the program. Each one has a variant for macOS, for
//! Windows and for the other systems (Linux, for CI) with the same names, so the code outside
//! this module has no `cfg` of a system.
//!
//! - The folders of the settings and of the logs, and the variables that name them.
//! - The lock of the process on a file of the data folder.
//! - The activity against App Nap of macOS ([`crate::nap`]).
//! - The notifications of the system ([`crate::notify`]).
//! - A start error: a dialog on Windows, whose release build has no console.

#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod system;
#[cfg(windows)]
#[path = "windows.rs"]
mod system;
#[cfg(not(any(target_os = "macos", windows)))]
#[path = "other.rs"]
mod system;

pub(crate) use self::system::{Activity, FOLDER_VARIABLES, Notifications, folders, show_start_error, try_lock};
