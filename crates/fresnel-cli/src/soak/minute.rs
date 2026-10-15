//! The record of one minute of the soak: what the core shows, what the checks found, the
//! heap and the open connections. The journal of the core goes to a file of the report.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
use std::path::PathBuf;
use std::time::Duration;

use fresnel_core::app::ClusterRow;
use fresnel_core::journal::JournalView;
use fresnel_core::net::{ChannelState, ClusterStatus};
use serde::Serialize;

use super::check::{Gap, Tally};
use super::logs::{LogCheck, LogCounts};
use super::look::Look;

/// One minute of the soak.
#[derive(Debug, Serialize)]
pub(super) struct Minute {
    pub(super) minute: u64,
    pub(super) time: String,
    /// The bytes on the heap of the process at the end of the minute.
    pub(super) heap_bytes: usize,
    /// The open file descriptors of the process, sockets among them. `None` where the
    /// system does not list them.
    pub(super) descriptors: Option<usize>,
    /// The one state of the cluster.
    pub(super) health: String,
    /// The connections of the core to the cluster that are up now: each channel that is
    /// connected, and the warm upgrade connections.
    pub(super) open_connections: usize,
    /// The connections that the channels opened since the start.
    pub(super) opened_connections: u64,
    pub(super) stores_live: usize,
    /// Each store that is not live, with its state.
    pub(super) stores_not_live: Vec<String>,
    /// The checks that ended in the minute.
    pub(super) checks: usize,
    pub(super) lag_median_ms: u64,
    pub(super) lag_max_ms: u64,
    pub(super) gaps: Vec<Gap>,
    /// The lists of the comparison that failed, by their reason.
    pub(super) failed_lists: BTreeMap<String, u64>,
    /// The state of the two lists of the metrics sampler.
    pub(super) metrics: Option<String>,
    pub(super) log: Option<LogCounts>,
    /// The state of each source of the log view.
    pub(super) log_sources: Vec<String>,
    pub(super) warnings: u64,
    pub(super) errors: u64,
    /// The faults that the harness made in the minute.
    pub(super) faults: Vec<String>,
}

/// What the minutes keep from one to the next.
pub(super) struct Recorder {
    journal: File,
    /// The number of the next journal entry to write, and the counts of the last minute.
    next_entry: u64,
    counts: (u64, u64),
    faults: Option<(PathBuf, u64)>,
    log: LogCheck,
}

impl Recorder {
    pub(super) fn new(journal: File, faults: Option<PathBuf>) -> Self {
        let faults = faults.map(|file| (file, 0));
        Self { journal, next_entry: 0, counts: (0, 0), faults, log: LogCheck::default() }
    }

    /// The record of one minute from a copy of the core and the tally of the checks.
    pub(super) fn record(&mut self, minute: u64, look: &Look, tally: Tally) -> Minute {
        let row = look.row.as_ref();
        let status = row.and_then(ClusterRow::status);
        let (live, not_live): (Vec<_>, Vec<_>) = look.statuses().iter().partition(|store| store.state.is_live());
        let stores_not_live = not_live.iter().map(|store| format!("{}: {}", store.kind.info().name, store.state));
        let mut lags: Vec<u64> = tally.lags.iter().copied().map(millis).collect();
        lags.sort_unstable();
        let data = row.and_then(|row| row.data.as_ref());
        let metrics = data.map(|data| {
            let metrics = &data.metrics;
            format!("pods {}; nodes {}; {} series", metrics.pods.state, metrics.nodes.state, metrics.series_count())
        });
        let view = data.and_then(|data| data.logs.first());
        let (warnings, errors) = self.write_journal(&look.journal);
        Minute {
            minute,
            time: now(),
            heap_bytes: crate::memory::live(),
            descriptors: descriptors(),
            health: row.map_or_else(|| "no cluster".to_owned(), |row| format!("{:?}", row.health())),
            open_connections: status.map_or(0, open),
            opened_connections: status.map_or(0, opened),
            stores_live: live.len(),
            stores_not_live: stores_not_live.collect(),
            checks: lags.len(),
            lag_median_ms: lags.get(lags.len() / 2).copied().unwrap_or_default(),
            lag_max_ms: lags.last().copied().unwrap_or_default(),
            gaps: tally.gaps,
            failed_lists: tally.failed,
            metrics,
            log: view.map(|view| self.log.read(view)),
            log_sources: view.map_or_else(Vec::new, |view| {
                view.sources.iter().map(|source| format!("{}: {}", source.pod.name, source.state)).collect()
            }),
            warnings,
            errors,
            faults: self.new_faults(),
        }
    }

    /// Writes the entries since the last minute to the journal file. Gives the warnings
    /// and the errors of the minute.
    fn write_journal(&mut self, journal: &JournalView) -> (u64, u64) {
        let new = journal.entries.iter().filter(|entry| entry.number >= self.next_entry);
        let text: String = new
            .map(|entry| format!("{} {:?} [{}] {}\n", entry.number, entry.level, entry.source, entry.text))
            .collect();
        if let Err(error) = self.journal.write_all(text.as_bytes()) {
            eprintln!("soak: cannot write the journal file: {error}");
        }
        self.next_entry = journal.total;
        let counts = (journal.warnings.saturating_sub(self.counts.0), journal.errors.saturating_sub(self.counts.1));
        self.counts = (journal.warnings, journal.errors);
        counts
    }

    /// The lines that the harness added to the file of the faults since the last minute.
    fn new_faults(&mut self) -> Vec<String> {
        let Some((file, read)) = &mut self.faults else {
            return Vec::new();
        };
        let mut text = String::new();
        let opened = File::open(&*file).and_then(|mut file| {
            file.seek(SeekFrom::Start(*read))?;
            file.read_to_string(&mut text)
        });
        match opened {
            Ok(bytes) => *read += bytes as u64,
            Err(error) => return vec![format!("the file of the faults cannot be read: {error}")],
        }
        text.lines().map(str::to_owned).collect()
    }
}

/// The time of now, to the second.
pub(super) fn now() -> String {
    jiff::Timestamp::now().round(jiff::Unit::Second).map_or_else(|_| String::new(), |time| time.to_string())
}

/// The channels of each purpose that are connected now, and the warm upgrade connections.
fn open(status: &ClusterStatus) -> usize {
    let groups = [&status.watches, &status.requests, &status.logs];
    let channels = groups.iter().flat_map(|group| &group.channels);
    channels.filter(|channel| matches!(channel.state, ChannelState::Live | ChannelState::Verifying)).count()
        + status.upgrade.warm
}

fn opened(status: &ClusterStatus) -> u64 {
    let groups = [&status.watches, &status.requests, &status.logs];
    groups.iter().flat_map(|group| &group.channels).map(|channel| channel.connections).sum()
}

/// The open file descriptors of the process, from the folder of the system that lists them.
/// The read of the folder holds one descriptor itself.
fn descriptors() -> Option<usize> {
    let entries = std::fs::read_dir("/dev/fd").ok()?;
    Some(entries.count().saturating_sub(1))
}

/// A lag in milliseconds.
fn millis(time: Duration) -> u64 {
    u64::try_from(time.as_millis()).unwrap_or(u64::MAX)
}
