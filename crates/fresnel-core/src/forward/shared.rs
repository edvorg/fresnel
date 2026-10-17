//! What the task of a forward shares with its local connections: the gate with the target
//! of now, the counters, and the outcomes of the tunnels that change the state.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime};

use fresnel_net::{ErrorClass, WallClock};
use tokio::sync::Notify;
use tokio::time::Instant;

use super::feed::Feed;
use super::resolve::TargetPod;
use super::view::{Bytes, Connections};
use crate::error::CoreError;
use crate::journal::Journal;

/// The time that a pod waits after the server did not find it: the store can be some time
/// behind a delete, and the next tunnel then goes to an other pod.
const SUSPECT_TIME: Duration = Duration::from_secs(10);

/// What a new local connection reads: the cluster and the pod of its tunnel. `None` while
/// no tunnel can open, and the connection waits.
#[derive(Clone, Default)]
pub(super) struct Gate {
    pub(super) target: Option<(Arc<Feed>, TargetPod)>,
}

/// What a local connection tells the task of its forward.
#[derive(Debug)]
pub(super) enum Outcome {
    /// A connection waits for a target: the cluster gets a retry now.
    Waits,
    /// A connection got a reset after its wait, for this reason.
    NoTarget(String),
    /// The open of a tunnel failed.
    OpenFailed { target: TargetPod, error: CoreError },
    /// A tunnel failed after its open. `path`: its connection broke, so the path can be lost.
    Broke { text: String, path: bool },
    /// A tunnel ended well: the failures in a row end.
    Clean,
}

/// The outcomes since the last read of the task.
#[derive(Debug, Default)]
pub(super) struct Reports {
    /// The tunnels that failed in a row, with no good one between them.
    pub(super) failures: u32,
    pub(super) last_error: Option<(SystemTime, String)>,
    /// The server refuses the forward: no retry runs.
    pub(super) refused: Option<String>,
    /// The UIDs of pods that the server did not find, each with the end of its wait.
    pub(super) suspects: Vec<(Arc<str>, Instant)>,
    /// A connection waits, or a tunnel broke: the cluster tests its path now.
    pub(super) retry: bool,
}

/// The counters of the connections and of the bytes.
#[derive(Debug, Default)]
pub(super) struct Counts {
    open: AtomicU64,
    total: AtomicU64,
    reset: AtomicU64,
    sent: AtomicU64,
    received: AtomicU64,
}

/// Counts one open connection while it lives, also when its task stops in a wait.
pub(super) struct Open<'a>(&'a Counts);

impl Drop for Open<'_> {
    fn drop(&mut self) {
        self.0.open.fetch_sub(1, Ordering::Relaxed);
    }
}

impl Counts {
    pub(super) fn accepted(&self) -> Open<'_> {
        self.open.fetch_add(1, Ordering::Relaxed);
        self.total.fetch_add(1, Ordering::Relaxed);
        Open(self)
    }

    pub(super) fn reset(&self) {
        self.reset.fetch_add(1, Ordering::Relaxed);
    }

    /// A connection above the limit: it counts in the total, and its end is a reset.
    pub(super) fn refused(&self) {
        self.total.fetch_add(1, Ordering::Relaxed);
        self.reset();
    }

    pub(super) fn sent(&self, bytes: usize) {
        self.sent.fetch_add(u64::try_from(bytes).unwrap_or(u64::MAX), Ordering::Relaxed);
    }

    pub(super) fn received(&self, bytes: usize) {
        self.received.fetch_add(u64::try_from(bytes).unwrap_or(u64::MAX), Ordering::Relaxed);
    }

    /// The values of now.
    pub(super) fn read(&self) -> (Connections, Bytes) {
        let read = |counter: &AtomicU64| counter.load(Ordering::Relaxed);
        let connections = Connections { open: read(&self.open), total: read(&self.total), reset: read(&self.reset) };
        (connections, Bytes { sent: read(&self.sent), received: read(&self.received) })
    }
}

/// What the task of a forward and its connections share.
pub(super) struct Shared {
    pub(super) counts: Counts,
    pub(super) journal: Journal,
    /// The longest wait of a new connection for a tunnel (plan section 12.3: 5 s).
    pub(super) wait: Duration,
    /// The time that a tunnel stays after the end of the local input while the pod is silent.
    pub(super) linger: Duration,
    /// The pause before a connection tries a tunnel again after a failure.
    pub(super) retry_pause: Duration,
    clock: WallClock,
    reports: Mutex<Reports>,
    /// Wakes the task for a new outcome.
    pub(super) changed: Notify,
}

impl Shared {
    pub(super) fn new(journal: Journal, times: [Duration; 3], clock: WallClock) -> Self {
        let [wait, linger, retry_pause] = times;
        let reports = Mutex::default();
        Self { counts: Counts::default(), journal, wait, linger, retry_pause, clock, reports, changed: Notify::new() }
    }

    /// Each use holds the lock for one small change or one copy, never across a wait.
    pub(super) fn reports(&self) -> MutexGuard<'_, Reports> {
        self.reports.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Takes the outcome of a connection, and wakes the task.
    pub(super) fn report(&self, outcome: Outcome) {
        let now = self.clock.now();
        {
            let mut reports = self.reports();
            match outcome {
                Outcome::Waits => reports.retry = true,
                Outcome::NoTarget(text) => reports.last_error = Some((now, text)),
                Outcome::OpenFailed { target, error } => {
                    let text = format!("a tunnel to pod {} did not open: {error}", target.pod.name);
                    match (error.class(), error.status()) {
                        (ErrorClass::Forbidden, _) => {
                            reports.refused = Some(format!("the server refuses the port forward: {error}"));
                        }
                        // The pod is gone or does not run: the store is behind.
                        (_, Some(400 | 404)) => reports.suspects.push((target.uid, Instant::now() + SUSPECT_TIME)),
                        (class, _) => {
                            reports.failures = reports.failures.saturating_add(1);
                            reports.retry |= matches!(class, ErrorClass::Network | ErrorClass::Timeout);
                        }
                    }
                    reports.last_error = Some((now, text));
                }
                Outcome::Broke { text, path } => {
                    reports.failures = reports.failures.saturating_add(1);
                    reports.retry |= path;
                    reports.last_error = Some((now, text));
                }
                Outcome::Clean => reports.failures = 0,
            }
        }
        self.changed.notify_one();
    }
}
