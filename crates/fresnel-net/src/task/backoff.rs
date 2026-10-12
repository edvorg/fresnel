//! The pause before a task starts again.

use std::time::Duration;

/// The pause before a task starts again. It grows with each run that ended early.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Backoff {
    /// The pause after the first run that ended early.
    pub first: Duration,
    /// The longest pause. Jitter never makes a pause longer than this.
    pub max: Duration,
    /// The part of a pause that jitter can remove, in percent, from 0 to 100.
    pub jitter_percent: u8,
    /// A run of this length or longer is not an early end. The next pause is `first` again.
    pub reset_after: Duration,
}

impl Default for Backoff {
    /// First values. The longest pause leaves time for a connect inside a recovery limit of 5 seconds.
    fn default() -> Self {
        Self {
            first: Duration::from_millis(100),
            max: Duration::from_secs(3),
            jitter_percent: 50,
            reset_after: Duration::from_secs(10),
        }
    }
}

impl Backoff {
    /// The pause after `early_ends` ends in sequence. The end of a long run counts as 1.
    ///
    /// `sample` is a random number. Jitter keeps tasks that failed for one cause from a restart
    /// at the same instant. The function has no path that can panic, because a panic here would
    /// stop the supervisor of a task.
    pub(crate) fn pause(&self, early_ends: u32, sample: u32) -> Duration {
        let doublings = early_ends.saturating_sub(1).min(31);
        let base = self.first.saturating_mul(1 << doublings).min(self.max);
        let cut_percent = sample % (u32::from(self.jitter_percent.min(100)) + 1);
        base.saturating_sub((base / 100).saturating_mul(cut_percent))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    fn backoff(jitter_percent: u8) -> Backoff {
        Backoff { first: 100 * MS, max: 3_000 * MS, jitter_percent, reset_after: Duration::from_secs(10) }
    }

    #[test]
    fn the_pause_doubles_and_stops_at_the_limit() {
        let pauses: Vec<_> = (1..=8).map(|early_ends| backoff(0).pause(early_ends, 7)).collect();
        let expected = [100, 200, 400, 800, 1_600, 3_000, 3_000, 3_000].map(|ms| ms * MS);
        assert_eq!(pauses, expected);
    }

    #[test]
    fn jitter_removes_its_part_at_most() {
        for sample in 0..500 {
            let pause = backoff(50).pause(3, sample);
            assert!((200 * MS..=400 * MS).contains(&pause), "sample {sample} gives {pause:?}");
        }
        let all: std::collections::BTreeSet<_> = (0..500).map(|sample| backoff(50).pause(3, sample)).collect();
        assert_eq!(all.len(), 51, "each percent step occurs");
    }

    #[test]
    fn extreme_values_do_not_panic() {
        let huge = Backoff { first: Duration::MAX, max: Duration::MAX, jitter_percent: u8::MAX, reset_after: MS };
        let pause = huge.pause(u32::MAX, u32::MAX);
        assert!(Duration::ZERO < pause && pause < Duration::MAX, "{pause:?}");
        assert_eq!(backoff(100).pause(0, 100), Duration::ZERO);
        assert_eq!(backoff(0).pause(u32::MAX, u32::MAX), 3_000 * MS);
    }
}
