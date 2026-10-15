//! The notifications of macOS (plan section 11.6): the core gives a notice for a failed
//! operation and for a cluster that stays lost while the window is hidden, and this module
//! shows it in the Notification Center.
//!
//! The notification center of macOS works only for a program in an app bundle: a program
//! with no bundle identifier, such as `cargo run`, would stop with an exception. Such a
//! program shows no notification; the journal has each notice in each case.

use std::sync::Arc;

use fresnel_core::app::{Notice, Notifier};

/// The function that shows a notice, after the question for the permission of the user.
pub fn notifier() -> Notifier {
    let system = macos::System::start();
    Arc::new(move |notice: Notice| system.show(&notice))
}

#[cfg(target_os = "macos")]
mod macos {
    use std::sync::atomic::{AtomicU64, Ordering};

    use block2::RcBlock;
    use fresnel_core::app::Notice;
    use objc2::runtime::Bool;
    use objc2_foundation::{NSBundle, NSError, NSString};
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNMutableNotificationContent, UNNotificationRequest, UNUserNotificationCenter,
    };

    /// The notification center of the app, when the program has a bundle.
    pub(super) struct System {
        bundled: bool,
        /// The number of the last notice: each one gets an identifier of its own, so a new
        /// notice does not replace an older one.
        last: AtomicU64,
    }

    impl System {
        /// Asks the user one time for the permission to show notifications. The answer
        /// comes later; a notice before it waits in the center of the system.
        pub(super) fn start() -> Self {
            let bundled = NSBundle::mainBundle().bundleIdentifier().is_some();
            if bundled {
                let answer = RcBlock::new(|_granted: Bool, _error: *mut NSError| {});
                let options = UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound;
                UNUserNotificationCenter::currentNotificationCenter()
                    .requestAuthorizationWithOptions_completionHandler(options, &answer);
            }
            Self { bundled, last: AtomicU64::new(0) }
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
            UNUserNotificationCenter::currentNotificationCenter()
                .addNotificationRequest_withCompletionHandler(&request, None);
        }
    }
}

/// The other systems show no notification: the journal has each notice.
#[cfg(not(target_os = "macos"))]
mod macos {
    use fresnel_core::app::Notice;

    pub(super) struct System;

    impl System {
        pub(super) fn start() -> Self {
            Self
        }

        pub(super) fn show(&self, _notice: &Notice) {}
    }
}
