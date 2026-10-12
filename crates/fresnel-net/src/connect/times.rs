//! The connect times of one cluster. They set the delay of the hedged connect.

use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use super::ConnectSettings;

/// The count of connects that the typical time comes from.
const KEPT: usize = 9;

/// The time of the last connects of one cluster: TCP plus TLS of the attempt that won.
pub(crate) struct ConnectTimes {
    recent: Mutex<VecDeque<Duration>>,
}

impl ConnectTimes {
    pub(crate) fn new() -> Self {
        Self { recent: Mutex::new(VecDeque::with_capacity(KEPT)) }
    }

    pub(crate) fn record(&self, time: Duration) {
        let mut recent = self.recent.lock().unwrap_or_else(PoisonError::into_inner);
        if recent.len() == KEPT {
            recent.pop_front();
        }
        recent.push_back(time);
    }

    /// The median. One attempt that was held and then won does not move it.
    pub(crate) fn typical(&self) -> Option<Duration> {
        let mut times: Vec<Duration> =
            self.recent.lock().unwrap_or_else(PoisonError::into_inner).iter().copied().collect();
        times.sort_unstable();
        times.get(times.len() / 2).copied()
    }

    /// The wait before one more connect attempt starts (plan section 8.4).
    ///
    /// A delay below the connect time of a far cluster starts a second attempt for each
    /// connect. Thus the delay follows the measured time.
    pub(crate) fn hedge_delay(&self, settings: &ConnectSettings) -> Duration {
        match self.typical() {
            Some(typical) => {
                let delay = typical.saturating_mul(settings.hedge_percent) / 100;
                delay.max(settings.hedge_min).min(settings.hedge_max)
            }
            None => settings.hedge_first,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    fn delay_after(times_ms: &[u32]) -> Duration {
        let times = ConnectTimes::new();
        for time in times_ms {
            times.record(*time * MS);
        }
        times.hedge_delay(&ConnectSettings::default())
    }

    #[test]
    fn the_delay_is_one_and_a_half_connect_times_between_its_limits() {
        assert_eq!(delay_after(&[]), 200 * MS, "the first connect of a cluster has no measured time");
        assert_eq!(delay_after(&[2]), 150 * MS, "a near cluster gets the shortest delay");
        assert_eq!(delay_after(&[200]), 300 * MS);
        assert_eq!(delay_after(&[507]), 760 * MS + MS / 2, "a far cluster does not get two attempts for each connect");
        assert_eq!(delay_after(&[900]), 1000 * MS);
    }

    #[test]
    fn one_slow_connect_does_not_move_the_delay_and_old_connects_go() {
        assert_eq!(delay_after(&[200, 19_000, 200]), 300 * MS);
        let times = ConnectTimes::new();
        for time in (0..20).map(|step| 100 + step * 20) {
            times.record(time * MS);
        }
        assert_eq!(times.typical(), Some(400 * MS), "the median of the last 9 connects, 320 to 480 ms");
    }
}
