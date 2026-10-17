//! The accept of the local connections of a forward (plan section 12.3): a pause after an
//! accept that failed, and the limit of the open connections (design rules 3 and 12).
//!
//! An accept that fails, for example with "too many open files", fails again at once. One
//! journal entry each 100 ms pushed each other entry out of the journal, so the pause doubles
//! from 100 ms to 2 s, and the journal gets the first failure at once and the next ones in
//! one entry a minute with their count. Each local connection holds a tunnel and an upgrade
//! connection (TCP and TLS) to the API server: one above the limit gets a reset at once, and
//! its entries follow the same rule, as a client can connect again in a loop.

use std::io;
use std::time::Duration;

use fresnel_net::local::{LocalListener, LocalStream};
use fresnel_net::task::Backoff;
use tokio::time::{Instant, sleep_until};

use super::rule::ForwardRule;
use super::shared::Counts;
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
    /// The open connections at most.
    limit: usize,
    /// The failed accepts in a row, and the end of the pause after the last one.
    failures: u32,
    resume: Option<Instant>,
    failed: Tally,
    /// The error of the last failed accept.
    error: String,
    refused: Tally,
}

impl Accepts {
    pub(super) fn new(limit: usize) -> Self {
        let (failed, refused) = (Tally::default(), Tally::default());
        Self { limit, failures: 0, resume: None, failed, error: String::new(), refused }
    }

    /// The next connection of a client, after the pause of a failed accept.
    pub(super) async fn next(&self, listener: &LocalListener) -> io::Result<LocalStream> {
        if let Some(resume) = self.resume {
            sleep_until(resume).await;
        }
        listener.accept().await
    }

    /// Takes the result of [`Accepts::next`] while `open` connections are open. Gives the
    /// connection to serve: none after a failure, and none above the limit, which gets a reset.
    pub(super) fn take(
        &mut self,
        accepted: io::Result<LocalStream>,
        open: usize,
        counts: &Counts,
    ) -> Option<LocalStream> {
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
        if open < self.limit {
            return Some(local);
        }
        counts.refused();
        self.refused.count = self.refused.count.saturating_add(1);
        // The drop resets the connection at once: no byte of the app is on the way to it.
        drop(local);
        None
    }

    /// Gives the journal each entry that is due.
    pub(super) fn note(&mut self, rule: &ForwardRule, journal: &Journal) {
        let now = Instant::now();
        let (error, max, limit) = (&self.error, PAUSES.max.as_secs(), self.limit);
        let failed = self.failed.due(now).map(|count| match count {
            1 => format!("an accept of a local connection failed: {error}; the next one waits up to {max} s"),
            _ => format!("{count} accepts of a local connection failed since the last entry, the last: {error}"),
        });
        let refused = self.refused.due(now).map(|count| match count {
            1 => format!("a new local connection got a reset: {limit} are open, the limit of the forward"),
            _ => format!("{count} new local connections got a reset since the last entry, at the limit of {limit}"),
        });
        for text in [failed, refused].into_iter().flatten() {
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
