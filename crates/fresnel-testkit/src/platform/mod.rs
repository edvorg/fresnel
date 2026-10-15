//! The differences of the systems in the test kit, with a variant for unix and for Windows:
//! the environment of a program that the user starts, a program file, and a file for its
//! owner only.

#[cfg(unix)]
#[path = "unix.rs"]
mod system;
#[cfg(windows)]
#[path = "windows.rs"]
mod system;

#[cfg(unix)]
pub use self::system::FINDER_PATH;
pub(crate) use self::system::{PROGRAM_EXTENSION, create_private_folder, desktop_env, make_program, write_private};
