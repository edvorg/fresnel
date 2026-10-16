//! The state of one socket, the counters of the sockets of a cluster, and the one end.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::SystemTime;

use tokio::sync::watch;

use super::{OnEnd, SocketEnd, SocketFailure, SocketState};
use crate::changes::Changed;

/// A copy of the counters of the sockets of one cluster, for the Connections view.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SocketCounts {
    /// The sockets that are open now.
    pub open: u64,
    /// The sockets that opened since the start.
    pub opened: u64,
    /// The sockets that ended with no fault: a close, or the end of the server.
    pub closed: u64,
    /// The sockets that broke.
    pub failed: u64,
    /// The breaks of a silent connection: no frame answered a ping. A part of `failed`.
    pub silent: u64,
    /// The sockets whose write waits now for a peer that takes no bytes.
    pub write_waits: u64,
    /// The bytes of the messages that the app sent.
    pub sent: u64,
    /// The bytes of the messages that the server sent.
    pub received: u64,
    /// Why the last socket broke.
    pub last_failure: Option<String>,
}

/// The counters of the sockets of one cluster. Each use holds the lock for one small change
/// or one copy. Only a change of a state tells the readers: a count of bytes does not.
pub(crate) struct Counts {
    counts: Mutex<SocketCounts>,
    changed: Changed,
}

impl Counts {
    pub(crate) fn new(changed: Changed) -> Self {
        Self { counts: Mutex::new(SocketCounts::default()), changed }
    }

    pub(crate) fn read(&self) -> SocketCounts {
        self.counts().clone()
    }

    pub(super) fn sent(&self, bytes: usize) {
        let mut counts = self.counts();
        counts.sent = counts.sent.saturating_add(u64::try_from(bytes).unwrap_or(u64::MAX));
    }

    pub(super) fn received(&self, bytes: usize) {
        let mut counts = self.counts();
        counts.received = counts.received.saturating_add(u64::try_from(bytes).unwrap_or(u64::MAX));
    }

    fn change(&self, change: impl FnOnce(&mut SocketCounts)) {
        change(&mut self.counts());
        self.changed.notify();
    }

    fn counts(&self) -> MutexGuard<'_, SocketCounts> {
        self.counts.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The state of one socket. It counts the socket as open from its start, and gives the end
/// one time: at the end of the loop, at a panic, or at its drop when the owner dropped the
/// socket and its task stopped.
pub(super) struct Board {
    state: watch::Sender<SocketState>,
    counts: Arc<Counts>,
    on_end: Option<OnEnd>,
    /// The state shows a write that waits, and the counter has it.
    waits: bool,
    ended: bool,
}

impl Board {
    pub(super) fn new(state: watch::Sender<SocketState>, counts: Arc<Counts>, on_end: Option<OnEnd>) -> Self {
        counts.change(|counts| {
            counts.open += 1;
            counts.opened += 1;
        });
        Self { state, counts, on_end, waits: false, ended: false }
    }

    pub(super) fn counts(&self) -> &Counts {
        &self.counts
    }

    /// True while the state shows a write that waits.
    pub(super) fn shows_waits(&self) -> bool {
        self.waits
    }

    /// A write waits since `since`, or no more with `None`. A close keeps its state.
    pub(super) fn waits(&mut self, since: Option<SystemTime>) {
        if self.ended || self.waits == since.is_some() {
            return;
        }
        self.waits = since.is_some();
        self.counts.change(|counts| match since {
            Some(_) => counts.write_waits += 1,
            None => counts.write_waits = counts.write_waits.saturating_sub(1),
        });
        self.state.send_modify(|state| match (since, &*state) {
            (Some(since), SocketState::Open) => *state = SocketState::WriteWaits { since },
            (None, SocketState::WriteWaits { .. }) => *state = SocketState::Open,
            _ => {}
        });
    }

    pub(super) fn closing(&self) {
        if !self.ended {
            self.state.send_replace(SocketState::Closing);
        }
    }

    /// The final state, the counters and the report of the end. Only the first call counts.
    pub(super) fn end(&mut self, end: SocketEnd) {
        if self.ended {
            return;
        }
        self.waits(None);
        self.ended = true;
        self.counts.change(|counts| {
            counts.open = counts.open.saturating_sub(1);
            if end.is_clean() {
                counts.closed += 1;
            } else {
                counts.failed += 1;
                counts.silent += u64::from(matches!(end, SocketEnd::Failed(SocketFailure::Silent { .. })));
                counts.last_failure = Some(end.to_string());
            }
        });
        self.state.send_replace(SocketState::Ended(end.clone()));
        if let Some(on_end) = self.on_end.take() {
            on_end(&end);
        }
    }
}

impl Drop for Board {
    fn drop(&mut self) {
        self.end(SocketEnd::Dropped);
    }
}
