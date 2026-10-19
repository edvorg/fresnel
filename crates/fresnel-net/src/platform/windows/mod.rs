//! The Windows variant of the platform module.
//!
//! - `tree`: a child in a job object that ends each process of the tree.
//! - `tcp`: the bind of a local listener, and the bytes that a local client did not take.

mod tcp;
mod tree;

use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use windows_sys::Win32::System::WindowsProgramming::{QueryInterruptTime, QueryUnbiasedInterruptTime};

pub(crate) use self::tcp::{own_port, share_port, unacknowledged};
pub(crate) use self::tree::{Started, Tree, start};
use crate::env::{EnvSettings, EnvSource, ProcessEnv};

/// The variable of the home folder: `C:\Users\<name>`.
pub(crate) const HOME: &str = "USERPROFILE";
/// No login shell runs: the environment of the process is complete (see [`environment`]).
pub const LOGIN_SHELL: bool = false;
/// None: the process has the full `PATH` of the user (see [`environment`]).
pub const FIXED_FOLDERS: &[&str] = &[];
/// None, for the same reason.
pub(crate) const HOME_FOLDERS: &[&str] = &[];
/// The extensions of a program when `PATHEXT` is absent: the default of Windows.
const PROGRAM_EXTENSIONS: &str = ".COM;.EXE;.BAT;.CMD";

/// The environment of the process. Explorer and the Start menu give each program the
/// variables of the user and of the system from the registry, so it is complete, and Windows
/// has no login shell that adds to it.
pub(crate) async fn environment(base: &ProcessEnv, _settings: &EnvSettings) -> (ProcessEnv, EnvSource) {
    (base.clone(), EnvSource::Process)
}

/// The folders in which kubectl looks for `.kube\config`, in its sequence (`homedir` of
/// client-go): `%HOME%`, which Git Bash sets, `%HOMEDRIVE%%HOMEPATH%` and `%USERPROFILE%`.
pub(crate) fn kube_homes(env: &ProcessEnv) -> Vec<PathBuf> {
    let set = |name| env.get(name).filter(|value| !value.is_empty()).map(OsStr::to_owned);
    let drive = set("HOMEDRIVE").zip(set("HOMEPATH")).map(|(mut drive, path)| {
        drive.push(path);
        drive
    });
    [set("HOME"), drive, set(HOME)].into_iter().flatten().map(PathBuf::from).collect()
}

/// The name of a variable as a key: Windows does not tell `Path` from `PATH`.
pub(crate) fn variable_key(name: &OsStr) -> OsString {
    name.to_ascii_uppercase()
}

/// True for a command with no folder, which goes through `PATH`.
pub(crate) fn is_bare(command: &str) -> bool {
    !command.contains(['/', '\\'])
}

/// The files that a command can name, as Go finds the plugin for kubectl: the name itself
/// when it has an extension, then the name with each extension of `PATHEXT`. Thus `doctl`
/// finds `doctl.exe`.
pub(crate) fn program_files(path: PathBuf, env: &ProcessEnv) -> Vec<PathBuf> {
    let extensions = env.get("PATHEXT").and_then(OsStr::to_str).unwrap_or(PROGRAM_EXTENSIONS);
    let mut files: Vec<PathBuf> = path.extension().map(|_| path.clone()).into_iter().collect();
    for extension in extensions.split(';').filter(|extension| !extension.is_empty()) {
        let mut name = path.clone().into_os_string();
        name.push(extension);
        files.push(name.into());
    }
    files
}

/// Windows has no mode bit for execution: the extension makes a file a program.
pub(crate) fn is_program(path: &Path) -> bool {
    path.is_file()
}

/// The time that the computer slept since its start. The monotonic clock of Windows
/// (`QueryPerformanceCounter`) counts a sleep, so the wall clock does not jump against it
/// (`detect::ClockJump`). The interrupt time of the system counts a sleep too, and its
/// unbiased variant does not: the difference is the time asleep.
pub(crate) fn time_asleep() -> Duration {
    let (mut total, mut awake) = (0_u64, 0_u64);
    // SAFETY: each call writes one integer through its pointer, which is valid for the call.
    // The second one fails only for a null pointer.
    let read = unsafe {
        QueryInterruptTime(&raw mut total);
        QueryUnbiasedInterruptTime(&raw mut awake)
    };
    if read == 0 {
        return Duration::ZERO;
    }
    // Units of 100 ns. The second count is a moment later, so it can be larger by that moment.
    Duration::from_nanos(total.saturating_sub(awake).saturating_mul(100))
}

/// None. The list of the interfaces of Windows needs a call with a buffer of linked records
/// for each look; the route of the default interface (`detect::network`) already tells a new
/// network, cable or VPN, and the keepalive ping finds each connection that a change broke.
pub(crate) fn interface_addresses() -> BTreeSet<(String, IpAddr)> {
    BTreeSet::new()
}

#[cfg(test)]
mod tests;
