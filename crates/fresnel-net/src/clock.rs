//! The wall clock of the app.
//!
//! The monotonic clock stops while the laptop sleeps. A token expires by the wall clock, so
//! each time of a credential is a wall clock time. A test replaces the clock, so that paused
//! time also moves it.

use std::fmt;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

/// One long sleep ends too late after a wake. Short steps with a read of the wall clock end
/// at the correct time.
const LONGEST_STEP: Duration = Duration::from_secs(15);

/// Gives the wall clock time.
#[derive(Clone)]
pub struct WallClock(Arc<dyn Fn() -> SystemTime + Send + Sync>);

impl WallClock {
    /// The clock of the system.
    pub fn system() -> Self {
        Self(Arc::new(SystemTime::now))
    }

    /// A clock that reads `now`. A test uses it.
    pub fn new(now: impl Fn() -> SystemTime + Send + Sync + 'static) -> Self {
        Self(Arc::new(now))
    }

    /// The time now.
    pub fn now(&self) -> SystemTime {
        (self.0)()
    }

    /// Sleeps until the wall clock shows `at`. `None` sleeps with no end.
    pub(crate) async fn sleep_until(&self, at: Option<SystemTime>) {
        let Some(at) = at else {
            return std::future::pending().await;
        };
        // An error of `duration_since` means that the time is in the past.
        while let Ok(left) = at.duration_since(self.now()) {
            if left.is_zero() {
                return;
            }
            tokio::time::sleep(left.min(LONGEST_STEP)).await;
        }
    }
}

impl fmt::Debug for WallClock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("WallClock")
    }
}

/// The time between two wall clock times. Zero when `later` is not later.
pub(crate) fn since(later: SystemTime, earlier: SystemTime) -> Duration {
    later.duration_since(earlier).unwrap_or_default()
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;

    /// A wall clock that moves with the paused time of tokio. It starts at a fixed time.
    pub(crate) fn paused_clock() -> WallClock {
        let start = tokio::time::Instant::now();
        WallClock::new(move || epoch() + start.elapsed())
    }

    /// The start time of [`paused_clock`].
    pub(crate) fn epoch() -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000)
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{epoch, paused_clock};
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn the_sleep_ends_at_the_wall_clock_time_and_not_before() {
        let clock = paused_clock();
        let started = tokio::time::Instant::now();
        clock.sleep_until(Some(epoch() + Duration::from_secs(100))).await;
        assert_eq!(started.elapsed(), Duration::from_secs(100));
        clock.sleep_until(Some(epoch())).await;
        assert_eq!(started.elapsed(), Duration::from_secs(100), "a time in the past does not wait");
    }

    /// The laptop sleeps: the wall clock jumps and the monotonic clock does not.
    #[tokio::test(start_paused = true)]
    async fn a_jump_of_the_wall_clock_ends_the_sleep_at_the_next_step() {
        let start = tokio::time::Instant::now();
        let clock = WallClock::new(move || {
            let jump =
                if start.elapsed() >= Duration::from_secs(20) { Duration::from_secs(3600) } else { Duration::ZERO };
            epoch() + start.elapsed() + jump
        });
        clock.sleep_until(Some(epoch() + Duration::from_secs(600))).await;
        assert_eq!(start.elapsed(), Duration::from_secs(30), "two steps of 15 s, not 600 s");
    }

    #[test]
    fn a_time_that_is_not_later_gives_zero() {
        assert_eq!(since(epoch(), epoch() + Duration::from_secs(5)), Duration::ZERO);
        assert_eq!(since(epoch() + Duration::from_secs(5), epoch()), Duration::from_secs(5));
    }
}
