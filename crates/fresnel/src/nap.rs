//! The macOS activity that keeps App Nap off (plan section 11.6).
//!
//! macOS slows the timers of an app whose window is hidden. The keepalive ping of a
//! channel would then be late, and a silent connection would show late. The app holds an
//! activity while a cluster is selected. The activity permits the sleep of an idle system.
//! The other systems have no App Nap (`platform`).

use crate::platform::Activity;

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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_guard_begins_the_activity_one_time_and_ends_it_one_time() {
        let mut guard = NapGuard::default();
        assert!(!guard.hold(false), "no activity to end");
        assert!(guard.hold(true));
        assert!(!guard.hold(true), "a second call begins no second activity");
        assert!(guard.hold(false));
        assert!(!guard.hold(false));
        // The drop of a guard that holds the activity ends it.
        assert!(guard.hold(true));
    }
}
