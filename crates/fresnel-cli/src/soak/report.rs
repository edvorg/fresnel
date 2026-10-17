//! The report of the soak: `soak.json` with each minute, and `soak.txt` with the verdict, the
//! parts of the memory that grew, and the minutes in which something occurred. Each minute
//! writes both files again, so a soak that stops early leaves its report.

use std::hint::black_box;
use std::path::Path;

use fresnel_core::memory::Scale as _;

use serde::Serialize;

use super::account;
use super::args::Options;
use super::environment::Outages;
use super::minute::Minute;
use super::verdict::{Rules, Verdict};
use crate::memory::Copies;

const MIB: f64 = 1024.0 * 1024.0;
/// The parts of the memory account that the text names: those that grew most.
const GROWN_PARTS: usize = 10;

/// The report of one run.
#[derive(Debug, Serialize)]
pub(super) struct Report {
    pub(super) context: String,
    pub(super) started: String,
    pub(super) planned_minutes: u64,
    pub(super) memory_limit_mib: u64,
    pub(super) warm_up_minutes: u64,
    /// The Deployment of the log view, `namespace/name`.
    pub(super) log: Option<String>,
    pub(super) minutes: Vec<Minute>,
    /// The failures of the soak itself, for example a task of the soak that ended.
    pub(super) problems: Vec<String>,
    pub(super) verdict: Verdict,
    /// The outages of the environment that the file of the faults told.
    #[serde(skip)]
    outages: Outages,
}

impl Report {
    pub(super) fn new(options: &Options, started: String) -> Self {
        Self {
            context: options.context.clone(),
            started,
            planned_minutes: options.minutes,
            memory_limit_mib: options.memory_limit_mib,
            warm_up_minutes: options.warm_up,
            log: options.log.as_ref().map(|(namespace, name)| format!("{namespace}/{name}")),
            minutes: Vec::new(),
            problems: Vec::new(),
            verdict: Verdict::default(),
            outages: Outages::default(),
        }
    }

    /// Adds a minute, and marks each minute with a part in an outage of the environment: an
    /// outage starts before the line that tells it, so an earlier minute can be in it. The
    /// minute gets the bytes of the earlier records, as a copy of them measures them.
    pub(super) fn push(&mut self, mut minute: Minute) {
        minute.records_bytes = Copies.bytes(&mut || drop(black_box(self.minutes.clone())));
        minute.faults.iter().for_each(|line| self.outages.read(line));
        self.minutes.push(minute);
        for minute in &mut self.minutes {
            let start = minute.epoch.saturating_sub(60 + minute.slept_seconds);
            minute.outage = self.outages.covers(start, minute.epoch);
        }
    }

    /// Decides the verdict from the minutes so far.
    pub(super) fn judge(&mut self) {
        let warm_up = usize::try_from(self.warm_up_minutes).unwrap_or(usize::MAX);
        let rules = Rules { warm_up, memory_limit_mib: self.memory_limit_mib, log: self.log.is_some() };
        self.verdict = Verdict::of(&self.minutes, &rules, &self.problems);
    }

    /// Writes `soak.json` and `soak.txt` into the folder.
    pub(super) fn write(&self, folder: &Path) -> Result<(), String> {
        let json = serde_json::to_string_pretty(self).map_err(|error| format!("the report as JSON: {error}"))?;
        let write = |file: &str, text: &str| {
            std::fs::write(folder.join(file), text).map_err(|error| format!("cannot write {file}: {error}"))
        };
        write("soak.json", &json)?;
        write("soak.txt", &self.text())
    }

    /// The short report: the verdict, its numbers, the parts of the memory that grew, and
    /// each minute in which something occurred.
    pub(super) fn text(&self) -> String {
        let verdict = &self.verdict;
        let result = if verdict.passed { "PASS".to_owned() } else { format!("FAIL: {}", verdict.reasons.join("; ")) };
        let (minutes, planned) = (self.minutes.len(), self.planned_minutes);
        let mut text =
            format!("soak of {} from {}: {minutes} of {planned} minutes\n{result}\n\n", self.context, self.started);
        self.numbers().iter().filter(|line| !line.is_empty()).for_each(|line| text.push_str(&format!("{line}\n")));
        let base = self.minutes.get(usize::try_from(self.warm_up_minutes).unwrap_or(usize::MAX).max(1) - 1);
        if let (Some(base), Some(last)) = (base, self.minutes.last()) {
            let grown = account::grown(&base.memory, &last.memory, GROWN_PARTS);
            let (from, to) = (base.minute, last.minute);
            text.push_str(&format!("\nthe parts of the memory that grew most from minute {from} to minute {to}:\n"));
            for change in grown {
                let (items, bytes) = (change.items, change.bytes);
                let (before, after) = (bytes.0 as f64 / 1024.0, bytes.1 as f64 / 1024.0);
                let name = &change.name;
                text.push_str(&format!("  {name}: {} to {} items, {before:.0} to {after:.0} KiB\n", items.0, items.1));
            }
        }
        text.push_str("\nminutes with a fault, a gap, a store that is not live, a lost line, an outage or a sleep:\n");
        for minute in self.minutes.iter().filter(|minute| is_notable(minute)) {
            line(&mut text, minute);
        }
        text
    }

    /// The lines of the numbers of the verdict.
    fn numbers(&self) -> Vec<String> {
        let verdict = &self.verdict;
        let mib = |bytes: f64| format!("{:.1} MiB", bytes / MIB);
        let heap = verdict.heap.as_ref().map_or_else(
            || "-".to_owned(),
            |heap| {
                let (base, growth) = (mib(heap.base_bytes as f64), mib(heap.growth_bytes as f64));
                format!("{base} at minute {}, growth after it {growth}", heap.base_minute)
            },
        );
        let last = self.minutes.last();
        vec![
            format!(
                "checks of a store against the API: {}, longest lag {} ms, most minutes in a row with none: {}",
                verdict.checks, verdict.lag_max_ms, verdict.unchecked_minutes
            ),
            format!(
                "gaps of 5 s or more: {}, with no visible state: {}, with no recovery: {}",
                verdict.gaps, verdict.gaps_without_state, verdict.gaps_stuck
            ),
            format!("faults of the harness: {}", verdict.faults),
            format!("floor of the heap with no buffer after the warm-up: {heap} (limit {} MiB)", self.memory_limit_mib),
            format!(
                "heap at the end: {}, open connections at the end: {}",
                mib(last.map_or(0.0, |minute| minute.heap_bytes as f64)),
                last.map_or(0, |minute| minute.open_connections)
            ),
            format!(
                "log lines that did not come: {}, that left the ring before a read: {}, most minutes in a row \
                 with no line: {}",
                verdict.lost_log_lines, verdict.unread_log_lines, verdict.log_quiet_minutes
            ),
            verdict.forward.as_ref().map_or_else(String::new, |forward| {
                format!(
                    "checks of the forward: {}, failed: {}, with no visible state: {}, closed port: {}",
                    forward.checks, forward.failed, forward.without_state, forward.port_closed
                )
            }),
            format!(
                "outages of the environment: {} minutes; in them, minutes with no check: {}, gaps with no \
                 recovery: {}, log lines that did not come: {}; the computer slept {} s",
                verdict.outage_minutes,
                verdict.outage_unchecked_minutes,
                verdict.outage_gaps_stuck,
                verdict.outage_lost_log_lines,
                verdict.slept_seconds
            ),
        ]
    }
}

fn is_notable(minute: &Minute) -> bool {
    let lost = minute.log.as_ref().is_some_and(|log| log.lost > 0);
    let forward = minute.forward.as_ref().is_some_and(|forward| forward.checks.iter().any(|check| !check.ok));
    let environment = minute.outage || minute.slept_seconds > 0;
    !minute.faults.is_empty()
        || !minute.gaps.is_empty()
        || !minute.stores_not_live.is_empty()
        || lost
        || forward
        || environment
}

/// One line of a minute, and one more line for each gap.
fn line(text: &mut String, minute: &Minute) {
    let log = minute.log.as_ref().map_or_else(String::new, |log| {
        format!(", log +{} lines {} markers {} lost", log.lines, log.markers, log.lost)
    });
    let faults =
        if minute.faults.is_empty() { String::new() } else { format!(", faults: {}", minute.faults.join(", ")) };
    let outage = if minute.outage { ", OUTAGE OF THE ENVIRONMENT" } else { "" };
    let slept = if minute.slept_seconds > 0 { format!(", slept {} s", minute.slept_seconds) } else { String::new() };
    text.push_str(&format!(
        "{:>5} {} heap {:.1} MiB (low {:.1}), {} connections, {} stores live, {} not live, lag {}/{} ms{log}{faults}{outage}{slept}\n",
        minute.minute,
        minute.time,
        minute.heap_bytes as f64 / MIB,
        minute.heap_low as f64 / MIB,
        minute.open_connections,
        minute.stores_live,
        minute.stores_not_live.len(),
        minute.lag_median_ms,
        minute.lag_max_ms,
    ));
    for gap in &minute.gaps {
        let state = if gap.without_state { "NO VISIBLE STATE" } else { "visible" };
        text.push_str(&format!(
            "      gap of {}: {:.1} s, live and behind {:.1} s ({state}); {}; states: {}\n",
            gap.kind,
            gap.seconds,
            gap.live_seconds,
            gap.reason,
            gap.states.join(", ")
        ));
    }
}

#[cfg(test)]
pub(super) mod tests;
