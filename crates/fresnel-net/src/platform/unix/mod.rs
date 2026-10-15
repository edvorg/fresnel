//! The unix variant of the platform module: macOS, and Linux for CI.
//!
//! - `shell`: the environment of the login shell.
//! - `tree`: a child in a new session and process group; `child`: its start with no fork.

mod child;
mod shell;
mod tree;

use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::net::IpAddr;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use nix::ifaddrs::getifaddrs;
use nix::net::if_::InterfaceFlags;

pub(crate) use self::shell::environment;
pub(crate) use self::tree::{Started, Tree, start};
use crate::env::ProcessEnv;

/// The variable of the home folder.
pub(crate) const HOME: &str = "HOME";
/// Where a credential plugin most often is, also when the login shell gave nothing.
pub const FIXED_FOLDERS: &[&str] = &["/opt/homebrew/bin", "/usr/local/bin"];
/// A fixed folder below the home folder, after the others.
pub(crate) const HOME_FOLDERS: &[&str] = &[".local/bin"];

/// The name of a variable as a key: unix tells `Path` from `PATH`.
pub(crate) fn variable_key(name: &OsStr) -> OsString {
    name.to_owned()
}

/// True for a command with no folder, which goes through `PATH`.
pub(crate) fn is_bare(command: &str) -> bool {
    !command.contains('/')
}

/// The files that a command can name: on unix, only the file of its name.
pub(crate) fn program_files(path: PathBuf, _env: &ProcessEnv) -> Vec<PathBuf> {
    vec![path]
}

/// A file with a mode bit for execution.
pub(crate) fn is_program(path: &Path) -> bool {
    path.metadata().is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

/// The time that the computer slept since its start: none here. The monotonic clock of macOS
/// and of Linux stops during a sleep, so the wall clock jumps against it (`detect::ClockJump`).
pub(crate) fn time_asleep() -> Duration {
    Duration::ZERO
}

/// Each address of each interface that is up, with the name of the interface. With an
/// error of the system, the list is empty.
pub(crate) fn interface_addresses() -> BTreeSet<(String, IpAddr)> {
    let addresses = getifaddrs().into_iter().flatten().filter_map(|interface| {
        let up = interface.flags.contains(InterfaceFlags::IFF_UP | InterfaceFlags::IFF_RUNNING);
        let address = interface.address?;
        let v4 = address.as_sockaddr_in().map(|v4| IpAddr::V4(v4.ip()));
        let v6 = address.as_sockaddr_in6().map(|v6| IpAddr::V6(v6.ip()));
        let ip = v4.or(v6).filter(|ip| up && !ip.is_loopback())?;
        Some((interface.interface_name, ip))
    });
    addresses.collect()
}
