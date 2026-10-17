//! The environment of the soak: an outage of the API server that the harness tells in the
//! file of the faults, and a sleep of the computer. In an outage, k3s itself is down and the
//! harness restores it; a check or a log line that cannot come then is no fault of the app.
//! The report tells these minutes apart, and the gap rule holds in them as in each other one.
//!
//! The harness writes `outage from <seconds>: <text>` and `outage to <seconds>: <text>`, with
//! the seconds of the wall clock since 1970. A minute is in an outage when a part of it is.

/// The seconds that two minutes of the soak can be apart on the wall clock beyond the minute
/// itself before the gap is a sleep of the computer: the minutes count awake time.
pub(super) const SLEEP_SECONDS: u64 = 15;

/// The outages that the lines of the faults told so far.
#[derive(Debug, Default)]
pub(super) struct Outages {
    /// The start and the end of each one; no end while it lasts.
    spans: Vec<(u64, Option<u64>)>,
}

impl Outages {
    /// Takes one line of the file of the faults. Each other line is a fault.
    pub(super) fn read(&mut self, line: &str) {
        let seconds = |rest: &str| rest.split(':').next().and_then(|number| number.trim().parse::<u64>().ok());
        if let Some(from) = line.strip_prefix("outage from ").and_then(seconds) {
            self.spans.push((from, None));
        } else if let Some(to) = line.strip_prefix("outage to ").and_then(seconds)
            && let Some((_, end)) = self.spans.last_mut().filter(|(_, end)| end.is_none())
        {
            *end = Some(to);
        }
    }

    /// True when an outage has a part of the time from `start` to `end`.
    pub(super) fn covers(&self, start: u64, end: u64) -> bool {
        self.spans.iter().any(|(from, to)| *from <= end && to.is_none_or(|to| to >= start))
    }
}

/// The seconds that the computer slept between two minutes that ended at these times of the
/// wall clock, in seconds since 1970.
pub(super) fn slept(before: Option<u64>, now: u64) -> u64 {
    let beyond = before.map_or(0, |before| now.saturating_sub(before).saturating_sub(60));
    if beyond > SLEEP_SECONDS { beyond } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_outage_covers_each_minute_with_a_part_in_it_and_one_that_lasts_covers_the_rest() {
        let mut outages = Outages::default();
        outages.read("a restart of the API server");
        outages.read("outage from 1000: the API server of k3s does not answer; a restart follows");
        assert!(outages.covers(990, 1050) && outages.covers(5000, 5060), "it lasts");
        outages.read("outage to 1200: the API server answers after a restart of the container");
        assert!(!outages.covers(880, 940) && outages.covers(940, 1000) && outages.covers(1200, 1260));
        assert!(!outages.covers(1201, 1261));
        outages.read("outage to 1300: a second end changes nothing");
        assert!(!outages.covers(1250, 1290));
    }

    #[test]
    fn a_jump_of_the_wall_clock_beyond_the_minute_is_a_sleep() {
        assert_eq!(slept(None, 1000), 0, "the first minute");
        assert_eq!(slept(Some(1000), 1060), 0);
        assert_eq!(slept(Some(1000), 1075), 0, "a late minute");
        assert_eq!(slept(Some(1000), 1076), 16);
        assert_eq!(slept(Some(1000), 1000 + 60 + 9 * 3600), 9 * 3600);
        assert_eq!(slept(Some(1000), 990), 0, "a clock that went back");
    }
}
