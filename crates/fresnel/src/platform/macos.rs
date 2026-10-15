//! The macOS variant of the platform module.

use std::fs::{File, TryLockError};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use block2::RcBlock;
use fresnel_core::app::Notice;
use fresnel_core::journal::Journal;
use fresnel_core::net::env::ProcessEnv;
use objc2::rc::Retained;
use objc2::runtime::{Bool, NSObjectProtocol, ProtocolObject};
use objc2_foundation::{NSActivityOptions, NSBundle, NSError, NSProcessInfo, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotificationRequest, UNUserNotificationCenter,
};

/// The variables that [`folders`] reads.
pub(crate) const FOLDER_VARIABLES: &str = "HOME";
const REASON: &str = "fresnel keeps the connections to the selected clusters";
/// The source of the journal entries of the notifications.
const SOURCE: &str = "notice";

/// The data folder and the log folder of the app in the library of the user.
pub(crate) fn folders(env: &ProcessEnv, name: &str) -> Option<(PathBuf, PathBuf)> {
    let library = PathBuf::from(env.home()?).join("Library");
    Some((library.join("Application Support").join(name), library.join("Logs").join(name)))
}

/// A lock of the whole file. The system frees it when the process ends.
pub(crate) fn try_lock(file: &File) -> Result<(), TryLockError> {
    file.try_lock()
}

/// One activity of the process. Its drop ends it.
#[derive(Debug)]
pub(crate) struct Activity {
    token: Retained<ProtocolObject<dyn NSObjectProtocol>>,
}

impl Activity {
    pub(crate) fn begin() -> Self {
        let options = NSActivityOptions::UserInitiatedAllowingIdleSystemSleep;
        let reason = NSString::from_str(REASON);
        Self { token: NSProcessInfo::processInfo().beginActivityWithOptions_reason(options, &reason) }
    }
}

impl Drop for Activity {
    fn drop(&mut self) {
        // SAFETY: the token is the object that `beginActivityWithOptions:reason:` gave,
        // and it ends one time: this value owns it.
        unsafe { NSProcessInfo::processInfo().endActivity(&self.token) };
    }
}

/// The notification center of the app, when the program has a bundle.
///
/// The notification center of macOS works only for a program in an app bundle: a program
/// with no bundle identifier, such as `cargo run`, would stop with an exception. Such a
/// program shows no notification; the journal has each notice in each case, and a refusal
/// of macOS too.
pub(crate) struct Notifications {
    bundled: bool,
    /// The number of the last notice: each one gets an identifier of its own, so a new
    /// notice does not replace an older one.
    last: AtomicU64,
    journal: Journal,
}

impl Notifications {
    /// Asks the user one time for the permission to show notifications. The answer
    /// comes later; a notice before it waits in the center of the system.
    pub(crate) fn start(journal: Journal) -> Self {
        let bundled = NSBundle::mainBundle().bundleIdentifier().is_some();
        if bundled {
            let refused = journal.clone();
            let answer = RcBlock::new(move |granted: Bool, _error: *mut NSError| {
                if !granted.as_bool() {
                    let text = "macOS shows no notification of Fresnel: System Settings, Notifications allows them";
                    refused.warning(SOURCE, text);
                }
            });
            let options = UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound;
            UNUserNotificationCenter::currentNotificationCenter()
                .requestAuthorizationWithOptions_completionHandler(options, &answer);
        }
        Self { bundled, last: AtomicU64::new(0), journal }
    }

    pub(crate) fn show(&self, notice: &Notice) {
        if !self.bundled {
            return;
        }
        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(&notice.title));
        content.setBody(&NSString::from_str(&notice.body));
        let number = self.last.fetch_add(1, Ordering::Relaxed) + 1;
        let identifier = NSString::from_str(&format!("fresnel-notice-{number}"));
        let request = UNNotificationRequest::requestWithIdentifier_content_trigger(&identifier, &content, None);
        let (journal, title) = (self.journal.clone(), notice.title.clone());
        let answer = RcBlock::new(move |error: *mut NSError| {
            if !error.is_null() {
                journal.warning(SOURCE, format!("macOS did not take the notification \"{title}\""));
            }
        });
        UNUserNotificationCenter::currentNotificationCenter()
            .addNotificationRequest_withCompletionHandler(&request, Some(&answer));
    }
}
