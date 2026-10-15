//! The report of the soak: `soak.json` with each minute, and `soak.txt` with the verdict
//! and the minutes in which something occurred. Each minute writes both files again, so a
//! soak that stops early leaves its report.

use std::path::Path;

use serde::Serialize;

use super::args::Options;
use super::check::{GIVE_UP, LIMIT};
use super::minute::Minute;

const MIB: f64 = 1024.0 * 1024.0;
/// The minutes in a row with no line of the log view, or with no check of a store that ended,
/// that fail the soak: the view stopped, or the soak compares nothing. The longest fault of the
/// harness, a break of 3 minutes, gives 3 such minutes at most: the pods of the log view write
/// two lines each second, a stream gets the lines of a break after it, and a check ends when
/// its store catches up after the break.
const QUIET: usize = 5;

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
}

/// The result of the run.
#[derive(Debug, Default, Serialize)]
pub(super) struct Verdict {
    pub(super) passed: bool,
    pub(super) reasons: Vec<String>,
    pub(super) checks: usize,
    /// The most minutes in a row in which no check ended, after the first check.
    pub(super) unchecked_minutes: usize,
    pub(super) gaps: usize,
    pub(super) gaps_without_state: usize,
    /// The gaps in which the store showed a state and did not catch up in `GIVE_UP`.
    pub(super) gaps_stuck: usize,
    pub(super) lag_max_ms: u64,
    /// The heap at the end of the warm-up, and its largest growth after it.
    pub(super) heap_base_bytes: Option<usize>,
    pub(super) heap_growth_bytes: Option<i64>,
    pub(super) lost_log_lines: u64,
    /// The most minutes in a row with no line of the log view, after its first line.
    pub(super) log_quiet_minutes: usize,
    pub(super) faults: usize,
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
        }
    }

    /// Decides the verdict from the minutes so far. A gap with no visible state, a store that
    /// does not recover, a heap that grows above the limit, a lost log line, a log view that
    /// stops or a failure of the soak itself fails it.
    pub(super) fn judge(&mut self) {
        let minutes = &self.minutes;
        let gaps = minutes.iter().flat_map(|minute| &minute.gaps);
        let lines: u64 = minutes.iter().filter_map(|minute| minute.log.as_ref()).map(|log| log.lines).sum();
        let mut verdict = Verdict {
            checks: minutes.iter().map(|minute| minute.checks).sum(),
            gaps: gaps.clone().count(),
            gaps_without_state: gaps.clone().filter(|gap| gap.without_state).count(),
            gaps_stuck: gaps.filter(|gap| gap.open && !gap.without_state).count(),
            lag_max_ms: minutes.iter().map(|minute| minute.lag_max_ms).max().unwrap_or_default(),
            lost_log_lines: minutes.iter().filter_map(|minute| minute.log.as_ref()).map(|log| log.lost).sum(),
            unchecked_minutes: longest_quiet(minutes, |minute| minute.checks == 0),
            log_quiet_minutes: longest_quiet(minutes, |minute| minute.log.as_ref().is_none_or(|log| log.lines == 0)),
            faults: minutes.iter().map(|minute| minute.faults.len()).sum(),
            ..Verdict::default()
        };
        let warm_up = usize::try_from(self.warm_up_minutes).unwrap_or(usize::MAX);
        if let Some(base) = minutes.get(warm_up.saturating_sub(1)).map(|minute| minute.heap_bytes) {
            let later = minutes.iter().skip(warm_up).map(|minute| minute.heap_bytes).max().unwrap_or(base);
            verdict.heap_base_bytes = Some(base);
            verdict.heap_growth_bytes =
                Some(i64::try_from(later).unwrap_or(i64::MAX) - i64::try_from(base).unwrap_or(0));
        }
        let limit = i64::try_from(self.memory_limit_mib).unwrap_or(i64::MAX) * 1024 * 1024;
        let reasons = &mut verdict.reasons;
        if verdict.checks == 0 {
            reasons.push("no check of a store ended".to_owned());
        } else if verdict.unchecked_minutes >= QUIET {
            let minutes = verdict.unchecked_minutes;
            reasons.push(format!("no check of a store ended for {minutes} minutes in a row: the lists failed"));
        }
        if verdict.gaps_without_state > 0 {
            let limit = LIMIT.as_secs();
            reasons.push(format!(
                "{} gaps: a store showed \"live\" and was behind for more than {limit} s",
                verdict.gaps_without_state
            ));
        }
        if verdict.gaps_stuck > 0 {
            let limit = GIVE_UP.as_secs();
            reasons.push(format!("{} gaps: a store was behind for {limit} s and did not recover", verdict.gaps_stuck));
        }
        if let Some(growth) = verdict.heap_growth_bytes.filter(|growth| *growth > limit) {
            reasons.push(format!(
                "the heap grew by {:.1} MiB after the warm-up; the limit is {} MiB",
                growth as f64 / MIB,
                self.memory_limit_mib
            ));
        }
        if verdict.lost_log_lines > 0 {
            reasons.push(format!("{} log lines did not come", verdict.lost_log_lines));
        }
        if self.log.is_some() && !minutes.is_empty() && lines == 0 {
            reasons.push("the log view gave no line".to_owned());
        } else if verdict.log_quiet_minutes >= QUIET {
            reasons.push(format!("the log view gave no line for {} minutes in a row", verdict.log_quiet_minutes));
        }
        reasons.extend(self.problems.iter().cloned());
        verdict.passed = reasons.is_empty();
        self.verdict = verdict;
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

    /// The short report: the verdict, its numbers, and each minute in which something occurred.
    pub(super) fn text(&self) -> String {
        let verdict = &self.verdict;
        let result = if verdict.passed { "PASS".to_owned() } else { format!("FAIL: {}", verdict.reasons.join("; ")) };
        let mut text = format!(
            "soak of {} from {}: {} of {} minutes\n{result}\n\n",
            self.context,
            self.started,
            self.minutes.len(),
            self.planned_minutes
        );
        let heap = |bytes: Option<usize>| {
            bytes.map_or_else(|| "-".to_owned(), |bytes| format!("{:.1} MiB", bytes as f64 / MIB))
        };
        let growth =
            verdict.heap_growth_bytes.map_or_else(|| "-".to_owned(), |bytes| format!("{:.1} MiB", bytes as f64 / MIB));
        let last = self.minutes.last();
        let lines = [
            format!(
                "checks of a store against the API: {}, longest lag {} ms, most minutes in a row with none: {}",
                verdict.checks, verdict.lag_max_ms, verdict.unchecked_minutes
            ),
            format!(
                "gaps of 5 s or more: {}, with no visible state: {}, with no recovery: {}",
                verdict.gaps, verdict.gaps_without_state, verdict.gaps_stuck
            ),
            format!("faults of the harness: {}", verdict.faults),
            format!(
                "heap after the warm-up: {}, growth after it: {growth} (limit {} MiB)",
                heap(verdict.heap_base_bytes),
                self.memory_limit_mib
            ),
            format!(
                "heap at the end: {}, open connections at the end: {}",
                heap(last.map(|minute| minute.heap_bytes)),
                last.map_or(0, |minute| minute.open_connections)
            ),
            format!(
                "log lines that did not come: {}, most minutes in a row with no line: {}",
                verdict.lost_log_lines, verdict.log_quiet_minutes
            ),
        ];
        lines.iter().for_each(|line| text.push_str(&format!("{line}\n")));
        text.push_str("\nminutes with a fault, a gap, a store that is not live or a lost line:\n");
        for minute in self.minutes.iter().filter(|minute| is_notable(minute)) {
            line(&mut text, minute);
        }
        text
    }
}

/// The most minutes in a row that are `quiet`, after the first one that is not: the checks
/// and the log view start after the first lists of the core, which can take minutes.
fn longest_quiet(minutes: &[Minute], quiet: impl Fn(&Minute) -> bool) -> usize {
    let (mut run, mut longest) = (0, 0);
    for minute in minutes.iter().skip_while(|minute| quiet(minute)) {
        run = if quiet(minute) { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    longest
}

fn is_notable(minute: &Minute) -> bool {
    let lost = minute.log.as_ref().is_some_and(|log| log.lost > 0);
    !minute.faults.is_empty() || !minute.gaps.is_empty() || !minute.stores_not_live.is_empty() || lost
}

/// One line of a minute, and one more line for each gap.
fn line(text: &mut String, minute: &Minute) {
    let log = minute.log.as_ref().map_or_else(String::new, |log| {
        format!(", log +{} lines {} markers {} lost", log.lines, log.markers, log.lost)
    });
    let faults =
        if minute.faults.is_empty() { String::new() } else { format!(", faults: {}", minute.faults.join(", ")) };
    text.push_str(&format!(
        "{:>5} {} heap {:.1} MiB, {} connections, {} stores live, {} not live, lag {}/{} ms{log}{faults}\n",
        minute.minute,
        minute.time,
        minute.heap_bytes as f64 / MIB,
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
mod tests {
    use std::path::PathBuf;

    use super::super::check::Gap;
    use super::super::logs::LogCounts;
    use super::*;

    const MIB_BYTES: usize = 1024 * 1024;

    fn report(warm_up: u64) -> Report {
        let options = Options {
            context: "k3s".to_owned(),
            kubeconfig: vec![PathBuf::from("/k")],
            compare: Vec::new(),
            report: PathBuf::from("/r"),
            minutes: 60,
            log: Some(("soak".to_owned(), "logger".to_owned())),
            memory_limit_mib: 64,
            warm_up,
            faults: None,
        };
        Report::new(&options, "2027-01-15T08:00:00Z".to_owned())
    }

    fn minute(number: u64, heap_mib: usize) -> Minute {
        Minute {
            minute: number,
            time: String::new(),
            heap_bytes: heap_mib * MIB_BYTES,
            descriptors: None,
            health: "Live".to_owned(),
            open_connections: 3,
            opened_connections: 3,
            stores_live: 39,
            stores_not_live: Vec::new(),
            checks: 234,
            lag_median_ms: 0,
            lag_max_ms: 12,
            gaps: Vec::new(),
            failed_lists: Default::default(),
            metrics: None,
            log: Some(LogCounts { lines: 120, ..LogCounts::default() }),
            log_sources: Vec::new(),
            warnings: 0,
            errors: 0,
            faults: Vec::new(),
        }
    }

    fn gap(without_state: bool) -> Gap {
        let states = vec!["waits for a connection".to_owned()];
        let reason = "ns/a is not in the store".to_owned();
        Gap { kind: "Pod", seconds: 30.0, live_seconds: 14.0, without_state, states, reason, open: false }
    }

    #[test]
    fn a_soak_with_visible_gaps_and_a_heap_inside_its_limit_passes() {
        let mut report = report(2);
        report.minutes = vec![minute(1, 200), minute(2, 10), minute(3, 70), minute(4, 40)];
        report.minutes[2].gaps.push(gap(false));
        report.minutes[2].faults.push("a break of the network of 30 s".to_owned());
        report.minutes[2].log = Some(LogCounts::default());
        report.judge();
        let verdict = &report.verdict;
        assert!(verdict.passed, "{:?}", verdict.reasons);
        assert_eq!((verdict.gaps, verdict.gaps_without_state, verdict.faults), (1, 0, 1));
        assert_eq!(verdict.log_quiet_minutes, 1, "a break of the network");
        assert_eq!(verdict.heap_growth_bytes, Some(60 * 1024 * 1024), "from the end of the warm-up, not the start");
        assert!(report.text().contains("gap of Pod: 30.0 s, live and behind 14.0 s (visible)"), "{}", report.text());
    }

    #[test]
    fn a_gap_with_no_visible_state_a_growth_over_the_limit_and_a_lost_line_each_fail_the_soak() {
        let mut report = report(1);
        report.minutes = vec![minute(1, 10), minute(2, 75)];
        report.minutes[1].gaps.push(gap(true));
        report.minutes[1].log = Some(LogCounts { lost: 3, ..LogCounts::default() });
        report.problems.push("task soak check Pod failed".to_owned());
        report.judge();
        assert!(!report.verdict.passed);
        let reasons = [
            "1 gaps: a store showed \"live\" and was behind for more than 20 s",
            "the heap grew by 65.0 MiB after the warm-up; the limit is 64 MiB",
            "3 log lines did not come",
            "task soak check Pod failed",
        ];
        assert_eq!(report.verdict.reasons, reasons);
        assert!(report.text().starts_with("soak of k3s from 2027-01-15T08:00:00Z: 2 of 60 minutes\nFAIL: 1 gaps"));
    }

    #[test]
    fn a_store_that_does_not_recover_and_a_log_view_that_stops_each_fail_the_soak() {
        let mut report = report(1);
        report.minutes = (1..=8).map(|number| minute(number, 10)).collect();
        report.minutes[1].gaps.push(Gap { open: true, ..gap(false) });
        report.minutes[3..].iter_mut().for_each(|minute| minute.log = Some(LogCounts::default()));
        report.judge();
        let reasons = [
            "1 gaps: a store was behind for 300 s and did not recover",
            "the log view gave no line for 5 minutes in a row",
        ];
        assert_eq!(report.verdict.reasons, reasons);
        assert_eq!((report.verdict.gaps_stuck, report.verdict.log_quiet_minutes), (1, 5));

        // The lists of the comparison fail for 5 minutes.
        report
            .minutes
            .iter_mut()
            .for_each(|minute| minute.log = Some(LogCounts { lines: 120, ..LogCounts::default() }));
        report.minutes[1].gaps.clear();
        report.minutes[2..7].iter_mut().for_each(|minute| minute.checks = 0);
        report.judge();
        assert_eq!(report.verdict.reasons, ["no check of a store ended for 5 minutes in a row: the lists failed"]);
        report.minutes[2..7].iter_mut().for_each(|minute| minute.checks = 234);

        // A view with no line at all; then one whose first lines come late, which passes.
        report.minutes.iter_mut().for_each(|minute| {
            minute.gaps.clear();
            minute.log = Some(LogCounts::default());
        });
        report.judge();
        assert_eq!(report.verdict.reasons, ["the log view gave no line"]);
        let lines = LogCounts { lines: 120, ..LogCounts::default() };
        report.minutes[6..].iter_mut().for_each(|minute| minute.log = Some(lines.clone()));
        report.judge();
        assert!(report.verdict.passed, "{:?}", report.verdict.reasons);
    }
}
