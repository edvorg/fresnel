//! The sleep detector: a jump of the wall clock against the monotonic clock.

use std::time::{Duration, SystemTime};

use tokio::time::Instant;

use super::{Detector, Disturbance};
use crate::clock::WallClock;

/// A smaller difference is a clock correction, not a sleep.
const SMALLEST_JUMP: Duration = Duration::from_secs(5);

/// Detects that the computer slept. The monotonic clock of macOS and of Linux stops during
/// a sleep, and the wall clock does not. Thus the timers of the app, also the keepalive
/// ping, start late after a wake, and this detector tells the app at once.
#[derive(Debug)]
pub struct ClockJump {
    clock: WallClock,
    smallest_jump: Duration,
    last: (SystemTime, Instant),
}

impl ClockJump {
    /// A detector on the clock of the system.
    pub fn system() -> Self {
        Self::new(WallClock::system(), SMALLEST_JUMP)
    }

    /// A detector on `clock`, for a jump of `smallest_jump` or more.
    pub fn new(clock: WallClock, smallest_jump: Duration) -> Self {
        let last = (clock.now(), Instant::now());
        Self { clock, smallest_jump, last }
    }
}

impl Detector for ClockJump {
    fn look(&mut self) -> Option<Disturbance> {
        let now = (self.clock.now(), Instant::now());
        let (before_wall, before_steady) = std::mem::replace(&mut self.last, now);
        // A wall clock that moved back gives an error here, and that is not a sleep.
        let wall = now.0.duration_since(before_wall).ok()?;
        let jump = wall.saturating_sub(now.1.duration_since(before_steady));
        (jump >= self.smallest_jump).then_some(Disturbance::Wake(jump))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicI64, Ordering};

    use tokio::time::sleep;

    use super::*;
    use crate::clock::testing::epoch;

    const SECOND: Duration = Duration::from_secs(1);

    /// A wall clock that follows the paused time plus an offset in seconds.
    fn clock() -> (WallClock, Arc<AtomicI64>) {
        let (start, offset) = (Instant::now(), Arc::new(AtomicI64::new(3600)));
        let moved = Arc::clone(&offset);
        let clock = WallClock::new(move || {
            let seconds = u64::try_from(moved.load(Ordering::SeqCst)).expect("a time after the epoch");
            epoch() + start.elapsed() + Duration::from_secs(seconds)
        });
        (clock, offset)
    }

    #[tokio::test(start_paused = true)]
    async fn a_jump_of_the_wall_clock_is_a_wake_and_normal_time_is_not() {
        let (clock, offset) = clock();
        let mut detector = ClockJump::new(clock, 5 * SECOND);
        for _ in 0..3 {
            sleep(SECOND).await;
            assert_eq!(detector.look(), None, "the two clocks moved the same time");
        }
        // The laptop sleeps for 10 minutes: only the wall clock moves.
        offset.fetch_add(600, Ordering::SeqCst);
        sleep(SECOND).await;
        assert_eq!(detector.look(), Some(Disturbance::Wake(600 * SECOND)));
        sleep(SECOND).await;
        assert_eq!(detector.look(), None, "one wake gives one event");
    }

    #[tokio::test(start_paused = true)]
    async fn a_small_correction_and_a_clock_that_moves_back_are_no_wake() {
        let (clock, offset) = clock();
        let mut detector = ClockJump::new(clock, 5 * SECOND);
        offset.fetch_add(4, Ordering::SeqCst);
        sleep(SECOND).await;
        assert_eq!(detector.look(), None, "a jump below the smallest jump");
        offset.fetch_sub(1800, Ordering::SeqCst);
        sleep(SECOND).await;
        assert_eq!(detector.look(), None, "the user set the clock back");
        // A task that got no processor time for a minute: the two clocks moved.
        sleep(60 * SECOND).await;
        assert_eq!(detector.look(), None);
        offset.fetch_add(5, Ordering::SeqCst);
        assert_eq!(detector.look(), Some(Disturbance::Wake(5 * SECOND)));
    }
}
