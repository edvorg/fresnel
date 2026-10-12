//! The state that the handle, the driver and the connections of one fake server share.

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, MutexGuard, PoisonError};

use bytes::Bytes;
use tokio::net::TcpListener;
use tokio::sync::{broadcast, watch};
use tokio::time::Instant;
use tokio_rustls::TlsAcceptor;

use super::FakeCounters;
use super::script::{Fault, FaultAnswer};
use super::seen::SeenRequest;
use super::watch::{FEED_EVENTS, Feed};

/// The request heads that the server keeps. The counter `requests` shows how many it saw.
const KEPT_REQUESTS: usize = 10_000;

pub(super) struct Shared {
    pub(super) listener: TcpListener,
    pub(super) tls: TlsAcceptor,
    /// The value of `SETTINGS_MAX_CONCURRENT_STREAMS` for each HTTP/2 connection.
    pub(super) stream_limit: u32,
    pub(super) counters: watch::Sender<FakeCounters>,
    pub(super) feed: broadcast::Sender<Feed>,
    /// Each change sends GOAWAY on each open HTTP/2 connection.
    pub(super) go_away: watch::Sender<u64>,
    script: Mutex<Script>,
}

/// What the test set, and what the server saw.
#[derive(Default)]
struct Script {
    faults: Vec<Fault>,
    bodies: HashMap<String, Bytes>,
    seen: VecDeque<SeenRequest>,
}

impl Shared {
    pub(super) fn new(listener: TcpListener, tls: TlsAcceptor, stream_limit: u32) -> Self {
        Self {
            listener,
            tls,
            stream_limit,
            counters: watch::Sender::new(FakeCounters::default()),
            feed: broadcast::Sender::new(FEED_EVENTS),
            go_away: watch::Sender::new(0),
            script: Mutex::new(Script::default()),
        }
    }

    pub(super) fn count(&self, change: impl FnOnce(&mut FakeCounters)) {
        self.counters.send_modify(change);
    }

    /// Counts a new connection and gives its number.
    pub(super) fn count_connection(&self) -> u64 {
        let mut number = 0;
        self.count(|counters| {
            number = counters.connections;
            counters.connections += 1;
        });
        number
    }

    pub(super) fn add_fault(&self, fault: Fault) {
        self.script().faults.push(fault);
    }

    pub(super) fn clear_faults(&self) {
        self.script().faults.clear();
    }

    /// The first fault in the sequence of `add_fault` that takes the request.
    pub(super) fn fault_for(&self, request: &SeenRequest) -> Option<FaultAnswer> {
        let now = Instant::now();
        self.script().faults.iter().find(|fault| fault.takes(request, now)).map(|fault| fault.answer)
    }

    pub(super) fn set_body(&self, path: &str, body: Bytes) {
        self.script().bodies.insert(path.to_owned(), body);
    }

    pub(super) fn body(&self, path: &str) -> Option<Bytes> {
        self.script().bodies.get(path).cloned()
    }

    /// Keeps the head of the request and counts it. The oldest head goes at the limit.
    pub(super) fn record(&self, request: SeenRequest) {
        {
            let mut script = self.script();
            if script.seen.len() == KEPT_REQUESTS {
                script.seen.pop_front();
            }
            script.seen.push_back(request);
        }
        self.count(|counters| counters.requests += 1);
    }

    pub(super) fn seen(&self) -> Vec<SeenRequest> {
        self.script().seen.iter().cloned().collect()
    }

    /// Each use holds the lock for one small change or one copy.
    fn script(&self) -> MutexGuard<'_, Script> {
        self.script.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
