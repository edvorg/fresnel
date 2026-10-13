//! The macOS activity that keeps App Nap off (plan section 11.6).
//!
//! macOS slows the timers of an app whose window is hidden. The keepalive ping of a
//! channel would then be late, and a silent connection would show late. The app holds an
//! activity while a cluster is selected. The activity permits the sleep of an idle system.

/// Holds the activity while it is wanted.
#[derive(Debug, Default)]
pub struct NapGuard {
    activity: Option<Activity>,
}

impl NapGuard {
    /// Begins the activity or ends it. Returns true when this call changed something.
    pub fn hold(&mut self, wanted: bool) -> bool {
        match (wanted, self.activity.is_some()) {
            (true, false) => self.activity = Some(Activity::begin()),
            (false, true) => self.activity = None,
            (true, true) | (false, false) => return false,
        }
        true
    }

    /// True while the activity is held.
    pub fn is_held(&self) -> bool {
        self.activity.is_some()
    }
}

#[cfg(target_os = "macos")]
use macos::Activity;

#[cfg(target_os = "macos")]
mod macos {
    use objc2::rc::Retained;
    use objc2::runtime::{NSObjectProtocol, ProtocolObject};
    use objc2_foundation::{NSActivityOptions, NSProcessInfo, NSString};

    const REASON: &str = "fresnel keeps the connections to the selected clusters";

    /// One activity of the process. Its drop ends it.
    #[derive(Debug)]
    pub(super) struct Activity {
        token: Retained<ProtocolObject<dyn NSObjectProtocol>>,
    }

    impl Activity {
        pub(super) fn begin() -> Self {
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
}

/// App Nap exists on macOS only.
#[cfg(not(target_os = "macos"))]
#[derive(Debug)]
struct Activity;

#[cfg(not(target_os = "macos"))]
impl Activity {
    fn begin() -> Self {
        Self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_guard_begins_the_activity_one_time_and_ends_it_one_time() {
        let mut guard = NapGuard::default();
        assert!(!guard.hold(false) && !guard.is_held());
        assert!(guard.hold(true) && guard.is_held());
        assert!(!guard.hold(true) && guard.is_held(), "a second call begins no second activity");
        assert!(guard.hold(false) && !guard.is_held());
        // The drop of a guard that holds the activity ends it.
        assert!(guard.hold(true));
    }
}
