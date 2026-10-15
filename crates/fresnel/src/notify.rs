//! The notifications of macOS (plan section 11.6): the core gives a notice for a failed
//! operation and for a cluster that stays lost while the window is hidden, and this module
//! shows it in the Notification Center.
//!
//! The notification center of macOS works only for a program in an app bundle: a program
//! with no bundle identifier, such as `cargo run`, would stop with an exception. Such a
//! program shows no notification; the journal has each notice in each case, and a refusal
//! of macOS too.

use std::sync::Arc;

use fresnel_core::app::{Notice, Notifier};
use fresnel_core::journal::Journal;

/// The function that shows a notice, after the question for the permission of the user.
/// `journal` takes each refusal of macOS.
pub fn notifier(journal: Journal) -> Notifier {
    let system = macos::System::start(journal);
    Arc::new(move |notice: Notice| system.show(&notice))
}

#[cfg(target_os = "macos")]
mod macos {
    use std::sync::atomic::{AtomicU64, Ordering};

    use block2::RcBlock;
    use fresnel_core::app::Notice;
    use fresnel_core::journal::Journal;
    use objc2::runtime::Bool;
    use objc2_foundation::{NSBundle, NSError, NSString};
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNMutableNotificationContent, UNNotificationRequest, UNUserNotificationCenter,
    };

    /// The source of the journal entries of the notifications.
    const SOURCE: &str = "notice";

    /// The notification center of the app, when the program has a bundle.
    pub(super) struct System {
        bundled: bool,
        /// The number of the last notice: each one gets an identifier of its own, so a new
        /// notice does not replace an older one.
        last: AtomicU64,
        journal: Journal,
    }

    impl System {
        /// Asks the user one time for the permission to show notifications. The answer
        /// comes later; a notice before it waits in the center of the system.
        pub(super) fn start(journal: Journal) -> Self {
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

        pub(super) fn show(&self, notice: &Notice) {
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
}

/// The other systems show no notification: the journal has each notice.
#[cfg(not(target_os = "macos"))]
mod macos {
    use fresnel_core::app::Notice;
    use fresnel_core::journal::Journal;

    pub(super) struct System;

    impl System {
        pub(super) fn start(_journal: Journal) -> Self {
            Self
        }

        pub(super) fn show(&self, _notice: &Notice) {}
    }
}
