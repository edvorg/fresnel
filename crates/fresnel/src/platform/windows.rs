//! The Windows variant of the platform module.

use std::fs::{File, TryLockError};
use std::io;
use std::os::windows::io::AsRawHandle as _;
use std::path::PathBuf;

use fresnel_core::app::Notice;
use fresnel_core::journal::Journal;
use fresnel_core::net::env::ProcessEnv;
use windows_sys::Win32::Foundation::ERROR_LOCK_VIOLATION;
use windows_sys::Win32::Storage::FileSystem::{LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY, LockFileEx};
use windows_sys::Win32::System::IO::OVERLAPPED;

/// The variables that [`folders`] and the folder of downloads read.
pub(crate) const FOLDER_VARIABLES: &str = "USERPROFILE, APPDATA or LOCALAPPDATA";
/// The place of the byte that the lock holds, 2 to the power 62, far after the end of each
/// lock file: the high and the low half of its offset.
const LOCKED_BYTE: (u32, u32) = (1 << 30, 0);

/// The data folder in the roaming application data of the user, which moves with a profile
/// to an other computer, and the log folder in the local application data.
pub(crate) fn folders(env: &ProcessEnv, name: &str) -> Option<(PathBuf, PathBuf)> {
    let folder = |variable| env.get(variable).filter(|value| !value.is_empty()).map(PathBuf::from);
    Some((folder("APPDATA")?.join(name), folder("LOCALAPPDATA")?.join(name).join("logs")))
}

/// A lock of one byte far after the end of the file. A lock of Windows stops each read of
/// its bytes through an other handle, so a lock of the whole file would hide the process id
/// from the second process. The system frees the lock when the process ends.
pub(crate) fn try_lock(file: &File) -> Result<(), TryLockError> {
    // SAFETY: the record has integers and a handle, for each of which zero is valid.
    let mut place: OVERLAPPED = unsafe { std::mem::zeroed() };
    (place.Anonymous.Anonymous.OffsetHigh, place.Anonymous.Anonymous.Offset) = LOCKED_BYTE;
    let flags = LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY;
    // SAFETY: the handle of the file is open, and `place` lives to the end of the call. The
    // handle is not for overlapped input, so the call does not keep the pointer.
    if unsafe { LockFileEx(file.as_raw_handle(), flags, 0, 1, 0, &raw mut place) } != 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(ERROR_LOCK_VIOLATION as i32) {
        Err(TryLockError::WouldBlock)
    } else {
        Err(TryLockError::Error(error))
    }
}

/// Windows has no App Nap: the timers of a hidden window keep their times.
#[derive(Debug)]
pub(crate) struct Activity;

impl Activity {
    pub(crate) fn begin() -> Self {
        Self
    }
}

/// No notification: a toast of Windows needs a shortcut of the app in the Start menu, which
/// no install makes yet. The journal has each notice, and it tells this at the start.
pub(crate) struct Notifications;

impl Notifications {
    pub(crate) fn start(journal: Journal) -> Self {
        journal.info("notice", "Windows shows no notification of Fresnel; the journal has each notice");
        Self
    }

    pub(crate) fn show(&self, _notice: &Notice) {}
}
