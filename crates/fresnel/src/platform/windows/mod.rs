//! The Windows variant of the platform module.
//!
//! - `front`: the window of a process, and its move to the front at a second start.

mod front;

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
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW, SW_SHOWNORMAL};

pub(crate) use self::front::show_first;

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
    let mut place = OVERLAPPED::default();
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

/// Shows why the program stops at its start in a dialog. A release build has no console
/// (`main.rs`), so the text on stderr shows nowhere, and a second start would end with no sign.
pub(crate) fn show_start_error(text: &str) {
    let wide = |text: &str| text.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let (text, title) = (wide(text), wide("Fresnel"));
    // SAFETY: each text ends with a zero and lives to the end of the call; the dialog has no
    // owner window. The answer is the button that closed it, and the program ends in each case.
    unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), MB_OK | MB_ICONERROR) };
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

/// Opens a URL in the browser of the user. The shell of Windows starts the browser: the app
/// starts no process itself.
pub(crate) fn open_url(url: &str) -> Result<(), String> {
    let wide = |text: &str| text.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let (verb, target) = (wide("open"), wide(url));
    // SAFETY: each text ends with a 0 and lives to the end of the call, and the call takes
    // no window and no parameters.
    let result = unsafe {
        ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), target.as_ptr(), std::ptr::null(), std::ptr::null(), {
            SW_SHOWNORMAL
        })
    };
    // A value above 32 tells a success; a smaller one is the error of the shell.
    match result as isize {
        code if code > 32 => Ok(()),
        code => Err(format!("the shell of Windows did not open it (error {code})")),
    }
}
