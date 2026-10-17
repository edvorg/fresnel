//! The verdict of the soak from its minutes. A gap with no visible state, a store that does
//! not recover, a heap that grows, a part above its limit, a lost log line, a log view or
//! checks that stop, or a failure of the soak itself fails it. In an outage of the
//! environment (`environment`), the checks and the lines that cannot come are counted apart.

use std::collections::BTreeSet;

use serde::Serialize;

use super::check::{GIVE_UP, LIMIT};
use super::forward;
use super::heap::{self, Heap};
use super::minute::Minute;

const MIB: f64 = 1024.0 * 1024.0;
/// The minutes in a row with no line of the log view, or with no check of a store that ended,
/// that fail the soak: the view stopped, or the soak compares nothing. The longest fault of the
/// harness, a break of 3 minutes, gives 3 such minutes at most: the pods of the log view write
/// lines all the time, a stream gets the lines of a break after it, and a check ends when its
/// store catches up after the break.
pub(super) const QUIET: usize = 5;

/// What the minutes are judged by.
pub(super) struct Rules {
    pub(super) warm_up: usize,
    /// The most growth of the floor of the heap with no buffer after the warm-up, in MiB.
    pub(super) memory_limit_mib: u64,
    /// True when the soak follows a log view.
    pub(super) log: bool,
}

/// The result of the run.
#[derive(Debug, Default, Serialize)]
pub(super) struct Verdict {
    pub(super) passed: bool,
    pub(super) reasons: Vec<String>,
    pub(super) checks: usize,
    /// The most minutes in a row in which no check ended, after the first check, with no
    /// minute of an outage of the environment.
    pub(super) unchecked_minutes: usize,
    pub(super) gaps: usize,
    pub(super) gaps_without_state: usize,
    /// The gaps in which the store showed a state and did not catch up in `GIVE_UP`, out of an
    /// outage of the environment.
    pub(super) gaps_stuck: usize,
    pub(super) lag_max_ms: u64,
    /// The heap after the warm-up. `None` before its end.
    pub(super) heap: Option<Heap>,
    /// Each part of the memory account that was above the limit of its buffer.
    pub(super) over_limits: Vec<String>,
    pub(super) lost_log_lines: u64,
    /// The lines that left the ring of the view before the soak read them: no number after
    /// them counts as lost, so the ring of the soak holds the lines of the longest break.
    pub(super) unread_log_lines: u64,
    /// The most minutes in a row with no line of the log view, after its first line, with no
    /// minute of an outage of the environment.
    pub(super) log_quiet_minutes: usize,
    pub(super) faults: usize,
    /// The checks of the forward. `None` for a soak with no forward.
    pub(super) forward: Option<forward::Verdict>,
    /// The environment: what failed in its outages, which fails no soak, and its sleeps.
    pub(super) outage_minutes: usize,
    pub(super) outage_unchecked_minutes: usize,
    pub(super) outage_gaps_stuck: usize,
    pub(super) outage_lost_log_lines: u64,
    pub(super) slept_seconds: u64,
}

impl Verdict {
    /// The verdict of the minutes so far, with the failures of the soak itself.
    pub(super) fn of(minutes: &[Minute], rules: &Rules, problems: &[String]) -> Self {
        let (outside, inside): (Vec<&Minute>, Vec<&Minute>) = minutes.iter().partition(|minute| !minute.outage);
        let gaps = minutes.iter().flat_map(|minute| &minute.gaps);
        let stuck = |minutes: &[&Minute]| -> usize {
            let gaps = minutes.iter().flat_map(|minute| &minute.gaps);
            gaps.filter(|gap| gap.open && !gap.without_state).count()
        };
        let lost = |minutes: &[&Minute]| -> u64 {
            minutes.iter().filter_map(|minute| minute.log.as_ref()).map(|log| log.lost).sum()
        };
        // The first minute of each part above its limit.
        let (mut over_limits, mut named) = (Vec::new(), BTreeSet::new());
        for (name, text) in minutes.iter().flat_map(|minute| heap::over_limits(&minute.memory)) {
            if named.insert(name) {
                over_limits.push(text);
            }
        }
        let mut verdict = Self {
            checks: minutes.iter().map(|minute| minute.checks).sum(),
            gaps: gaps.clone().count(),
            gaps_without_state: gaps.filter(|gap| gap.without_state).count(),
            gaps_stuck: stuck(&outside),
            lag_max_ms: minutes.iter().map(|minute| minute.lag_max_ms).max().unwrap_or_default(),
            heap: heap::heap(minutes, rules.warm_up),
            over_limits,
            lost_log_lines: lost(&outside),
            unread_log_lines: minutes.iter().filter_map(|minute| minute.log.as_ref()).map(|log| log.unread).sum(),
            unchecked_minutes: longest_quiet(minutes, |minute| minute.checks == 0),
            log_quiet_minutes: longest_quiet(minutes, |minute| minute.log.as_ref().is_none_or(|log| log.lines == 0)),
            faults: minutes.iter().map(|minute| minute.faults.len()).sum(),
            outage_minutes: inside.len(),
            outage_unchecked_minutes: inside.iter().filter(|minute| minute.checks == 0).count(),
            outage_gaps_stuck: stuck(&inside),
            outage_lost_log_lines: lost(&inside),
            slept_seconds: minutes.iter().map(|minute| minute.slept_seconds).sum(),
            ..Self::default()
        };
        let mut reasons = verdict.reasons(minutes, rules);
        if minutes.iter().any(|minute| minute.forward.is_some()) {
            let of_minutes: Vec<(Option<&forward::Minute>, bool)> =
                minutes.iter().map(|minute| (minute.forward.as_ref(), minute.outage)).collect();
            let (forward, why) = forward::Verdict::of(&of_minutes, QUIET);
            verdict.forward = Some(forward);
            reasons.extend(why);
        }
        reasons.extend(problems.iter().cloned());
        verdict.passed = reasons.is_empty();
        verdict.reasons = reasons;
        verdict
    }

    fn reasons(&self, minutes: &[Minute], rules: &Rules) -> Vec<String> {
        let mut reasons = Vec::new();
        if self.checks == 0 {
            reasons.push("no check of a store ended".to_owned());
        } else if self.unchecked_minutes >= QUIET {
            let minutes = self.unchecked_minutes;
            reasons.push(format!("no check of a store ended for {minutes} minutes in a row: the lists failed"));
        }
        if self.gaps_without_state > 0 {
            let (gaps, limit) = (self.gaps_without_state, LIMIT.as_secs());
            reasons.push(format!("{gaps} gaps: a store showed \"live\" and was behind for more than {limit} s"));
        }
        if self.gaps_stuck > 0 {
            let limit = GIVE_UP.as_secs();
            reasons.push(format!("{} gaps: a store was behind for {limit} s and did not recover", self.gaps_stuck));
        }
        let limit = i64::try_from(rules.memory_limit_mib).unwrap_or(i64::MAX) * 1024 * 1024;
        if let Some(heap) = self.heap.as_ref().filter(|heap| heap.growth_bytes > limit) {
            reasons.push(format!(
                "the floor of the heap with no buffer grew by {:.1} MiB after the warm-up, at minute {}; the limit is {} MiB",
                heap.growth_bytes as f64 / MIB,
                heap.growth_minute,
                rules.memory_limit_mib
            ));
        }
        reasons.extend(self.over_limits.iter().map(|part| format!("a part above the limit of its buffer: {part}")));
        if self.lost_log_lines > 0 {
            reasons.push(format!("{} log lines did not come", self.lost_log_lines));
        }
        let lines: u64 = minutes.iter().filter_map(|minute| minute.log.as_ref()).map(|log| log.lines).sum();
        if rules.log && !minutes.is_empty() && lines == 0 {
            reasons.push("the log view gave no line".to_owned());
        } else if self.log_quiet_minutes >= QUIET {
            reasons.push(format!("the log view gave no line for {} minutes in a row", self.log_quiet_minutes));
        }
        reasons
    }
}

/// The most minutes in a row that are `quiet`, after the first one that is not: the checks
/// and the log view start after the first lists of the core, which can take minutes. A minute
/// of an outage of the environment ends a run, as one that is not quiet.
fn longest_quiet(minutes: &[Minute], quiet: impl Fn(&Minute) -> bool) -> usize {
    let (mut run, mut longest) = (0, 0);
    for minute in minutes.iter().skip_while(|minute| quiet(minute)) {
        run = if quiet(minute) && !minute.outage { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    longest
}
