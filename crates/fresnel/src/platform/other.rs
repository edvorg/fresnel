//! The variant of the platform module for the other systems: Linux, on which CI runs the
//! tests. It has no App Nap, no notifications and no browser.

use std::fs::{File, TryLockError};
use std::path::PathBuf;

use fresnel_core::app::Notice;
use fresnel_core::journal::Journal;
use fresnel_core::net::env::ProcessEnv;

/// The variables that [`folders`] reads.
pub(crate) const FOLDER_VARIABLES: &str = "HOME";

/// The data folder in `.config` and the log folder in `.local/state` of the home folder.
pub(crate) fn folders(env: &ProcessEnv, name: &str) -> Option<(PathBuf, PathBuf)> {
    let home = PathBuf::from(env.home()?);
    Some((home.join(".config").join(name), home.join(".local/state").join(name).join("logs")))
}

/// A lock of the whole file. The system frees it when the process ends.
pub(crate) fn try_lock(file: &File) -> Result<(), TryLockError> {
    file.try_lock()
}

/// Nothing besides stderr: this variant is for CI on Linux, whose log shows stderr.
pub(crate) fn show_start_error(_text: &str) {}

/// False: this variant is for CI on Linux, which opens no window.
pub(crate) fn show_first(_process: u32) -> bool {
    false
}

/// App Nap exists on macOS only.
#[derive(Debug)]
pub(crate) struct Activity;

impl Activity {
    pub(crate) fn begin() -> Self {
        Self
    }
}

/// No notification: the journal has each notice.
pub(crate) struct Notifications;

impl Notifications {
    pub(crate) fn start(_journal: Journal) -> Self {
        Self
    }

    pub(crate) fn show(&self, _notice: &Notice) {}
}

/// No browser: the app starts no helper process, and Linux opens a URL with one.
pub(crate) fn open_url(_url: &str) -> Result<(), String> {
    Err("the app opens no browser on this system: copy the address".to_owned())
}
