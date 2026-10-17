//! The pace of the soak: the time between two changes, the lines of the logger, the pauses
//! between two faults, and a recreate of the cluster. `--fast` puts the changes, the lines
//! and the faults of hours into one hour, with one recreate in the middle of the run, so a
//! part of the core that grows shows in a short soak.

use std::time::Duration;

pub(crate) struct Pace {
    /// The time between two changes of the churn.
    pub(crate) step: Duration,
    /// The pause of the logger between two lines, as `sleep` of busybox takes it.
    pub(crate) line_pause: &'static str,
    /// The shortest and the longest time between two faults, in seconds.
    pub(crate) fault_pause: (u64, u64),
    /// When the faults recreate the cluster, from the start of the soak.
    pub(crate) recreate_after: Option<Duration>,
}

impl Pace {
    /// One change each 2 s, two lines each second, one fault each 2 to 6 minutes.
    pub(crate) const NORMAL: Self =
        Self { step: Duration::from_secs(2), line_pause: "0.5", fault_pause: (120, 360), recreate_after: None };

    /// Four times the changes and the faults, ten times the lines, and one recreate.
    pub(crate) fn fast(minutes: u64) -> Self {
        Self {
            step: Duration::from_millis(500),
            line_pause: "0.05",
            fault_pause: (30, 90),
            recreate_after: Some(Duration::from_secs(minutes * 60 / 2)),
        }
    }
}
