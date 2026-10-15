//! The report of the soak: `soak.json` with each minute, and `soak.txt` with the verdict
//! and the minutes in which something occurred. Each minute writes both files again, so a
//! soak that stops early leaves its report.

use std::path::Path;

use serde::Serialize;

use super::args::Options;
use super::check::LIMIT;
use super::minute::Minute;

const MIB: f64 = 1024.0 * 1024.0;

/// The report of one run.
#[derive(Debug, Serialize)]
pub(super) struct Report {
    pub(super) context: String,
    pub(super) started: String,
    pub(super) planned_minutes: u64,
    pub(super) memory_limit_mib: u64,
    pub(super) warm_up_minutes: u64,
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
    pub(super) gaps: usize,
    pub(super) gaps_without_state: usize,
    pub(super) lag_max_ms: u64,
    /// The heap at the end of the warm-up, and its largest growth after it.
    pub(super) heap_base_bytes: Option<usize>,
    pub(super) heap_growth_bytes: Option<i64>,
    pub(super) lost_log_lines: u64,
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
            minutes: Vec::new(),
            problems: Vec::new(),
            verdict: Verdict::default(),
        }
    }

    /// Decides the verdict from the minutes so far. A gap with no visible state, a heap that
    /// grows above the limit, a lost log line or a failure of the soak itself fails it.
    pub(super) fn judge(&mut self) {
        let minutes = &self.minutes;
        let gaps = minutes.iter().flat_map(|minute| &minute.gaps);
        let mut verdict = Verdict {
            checks: minutes.iter().map(|minute| minute.checks).sum(),
            gaps: gaps.clone().count(),
            gaps_without_state: gaps.filter(|gap| gap.without_state).count(),
            lag_max_ms: minutes.iter().map(|minute| minute.lag_max_ms).max().unwrap_or_default(),
            lost_log_lines: minutes.iter().filter_map(|minute| minute.log.as_ref()).map(|log| log.lost).sum(),
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
        }
        if verdict.gaps_without_state > 0 {
            let limit = LIMIT.as_secs();
            reasons.push(format!(
                "{} gaps: a store showed \"live\" and was behind for more than {limit} s",
                verdict.gaps_without_state
            ));
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
            format!("checks of a store against the API: {}, longest lag {} ms", verdict.checks, verdict.lag_max_ms),
            format!("gaps of 5 s or more: {}, with no visible state: {}", verdict.gaps, verdict.gaps_without_state),
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
            format!("log lines that did not come: {}", verdict.lost_log_lines),
        ];
        lines.iter().for_each(|line| text.push_str(&format!("{line}\n")));
        text.push_str("\nminutes with a fault, a gap, a store that is not live or a lost line:\n");
        for minute in self.minutes.iter().filter(|minute| is_notable(minute)) {
            line(&mut text, minute);
        }
        text
    }
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
