//! The accept of the local connections of a forward (plan section 12.3), with a pause after an
//! accept that failed.
//!
//! An accept that fails, for example with "too many open files", fails again at once. One
//! journal entry each 100 ms pushed each other entry out of the journal, so the pause doubles
//! from 100 ms to 2 s, and the journal gets the first failure at once and the next ones in
//! one entry a minute with their count.

use std::io;
use std::time::Duration;

use fresnel_net::local::{LocalListener, LocalStream};
use fresnel_net::task::Backoff;
use tokio::time::{Instant, sleep_until};

use super::rule::ForwardRule;
use crate::journal::Journal;

/// The pause after a failed accept, doubled after each failure in a row. `reset_after` is for
/// the runs of a task: here an accept that works ends the failures in a row.
const PAUSES: Backoff = Backoff {
    first: Duration::from_millis(100),
    max: Duration::from_secs(2),
    jitter_percent: 0,
    reset_after: Duration::ZERO,
};
/// The shortest time between two entries of one fault: the later one gives the count.
const ENTRY_GAP: Duration = Duration::from_secs(60);

/// The accepts of the task of one forward.
pub(super) struct Accepts {
    /// The failed accepts in a row, and the end of the pause after the last one.
    failures: u32,
    resume: Option<Instant>,
    failed: Tally,
    /// The error of the last failed accept.
    error: String,
}

impl Accepts {
    pub(super) fn new() -> Self {
        Self { failures: 0, resume: None, failed: Tally::default(), error: String::new() }
    }

    /// The next connection of a client, after the pause of a failed accept.
    pub(super) async fn next(&self, listener: &LocalListener) -> io::Result<LocalStream> {
        if let Some(resume) = self.resume {
            sleep_until(resume).await;
        }
        listener.accept().await
    }

    /// Takes the result of [`Accepts::next`]. Gives the connection to serve, none after a
    /// failure.
    pub(super) fn take(&mut self, accepted: io::Result<LocalStream>) -> Option<LocalStream> {
        let local = match accepted {
            Ok(local) => local,
            Err(error) => {
                self.failures = self.failures.saturating_add(1);
                self.resume = Some(Instant::now() + PAUSES.pause(self.failures, 0));
                (self.failed.count, self.error) = (self.failed.count.saturating_add(1), error.to_string());
                return None;
            }
        };
        (self.failures, self.resume) = (0, None);
        Some(local)
    }

    /// Gives the journal each entry that is due.
    pub(super) fn note(&mut self, rule: &ForwardRule, journal: &Journal) {
        let (error, max) = (&self.error, PAUSES.max.as_secs());
        let failed = self.failed.due(Instant::now()).map(|count| match count {
            1 => format!("an accept of a local connection failed: {error}; the next one waits up to {max} s"),
            _ => format!("{count} accepts of a local connection failed since the last entry, the last: {error}"),
        });
        if let Some(text) = failed {
            journal.warning(rule.cluster.as_str(), format!("{rule}: {text}"));
        }
    }
}

/// The faults of one kind since the last entry of the journal for them.
#[derive(Default)]
struct Tally {
    count: u64,
    /// The time of the last entry.
    noted: Option<Instant>,
}

impl Tally {
    /// The count for an entry, at most one in [`ENTRY_GAP`].
    fn due(&mut self, now: Instant) -> Option<u64> {
        if self.count == 0 || self.noted.is_some_and(|noted| now < noted + ENTRY_GAP) {
            return None;
        }
        self.noted = Some(now);
        Some(std::mem::take(&mut self.count))
    }
}

#[cfg(test)]
mod tests;
