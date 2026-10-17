//! The state that the handle, the driver and the connections of one fake server share.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use bytes::Bytes;
use http::header::AUTHORIZATION;
use tokio::net::TcpListener;
use tokio::sync::{broadcast, watch};
use tokio::time::Instant;
use tokio_rustls::TlsAcceptor;

use super::FakeCounters;
use super::pod;
use super::script::Fault;
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
    /// True: an HTTP/2 connection ends after the answer to a request that the token check
    /// of the server does not pass.
    pub(super) ends_unauthenticated: AtomicBool,
    /// The subprotocols of exec and attach that the fake pod speaks.
    exec_protocols: Mutex<&'static [&'static str]>,
    /// True while the WebSocket peers read nothing.
    streams_held: watch::Sender<bool>,
    /// True while a node starts no new port forward.
    forwards_muted: watch::Sender<bool>,
    script: Mutex<Script>,
}

/// One connection in the counters.
///
/// The connection counts as open until this value drops. The task of the connection owns
/// it, so the count is also correct after an abort or a panic of that task.
pub(super) struct Connection {
    /// The number of the connection in the sequence of accepts.
    pub(super) number: u64,
    pub(super) shared: Arc<Shared>,
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.shared.count(|counters| counters.open_connections = counters.open_connections.saturating_sub(1));
    }
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
            ends_unauthenticated: AtomicBool::new(false),
            exec_protocols: Mutex::new(&[pod::V5, pod::V4]),
            streams_held: watch::Sender::new(false),
            forwards_muted: watch::Sender::new(false),
            script: Mutex::new(Script::default()),
        }
    }

    pub(super) fn exec_protocols(&self) -> &'static [&'static str] {
        *self.exec_protocols.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(super) fn set_exec_protocols(&self, protocols: &'static [&'static str]) {
        *self.exec_protocols.lock().unwrap_or_else(PoisonError::into_inner) = protocols;
    }

    pub(super) fn hold_streams(&self, held: bool) {
        self.streams_held.send_replace(held);
    }

    /// Waits until the streams flow. The sender lives in `self`, so the wait cannot fail.
    pub(super) async fn until_streams_flow(&self) {
        let mut held = self.streams_held.subscribe();
        if held.wait_for(|held| !held).await.is_err() {
            std::future::pending::<()>().await;
        }
    }

    /// Waits until the test holds the streams.
    pub(super) async fn until_streams_held(&self) {
        let mut held = self.streams_held.subscribe();
        if held.wait_for(|held| *held).await.is_err() {
            std::future::pending::<()>().await;
        }
    }

    pub(super) fn mute_forwards(&self, muted: bool) {
        self.forwards_muted.send_replace(muted);
    }

    /// Waits until a node starts the streams of a port forward. The sender lives in `self`,
    /// so the wait cannot fail.
    pub(super) async fn until_forwards_start(&self) {
        let mut muted = self.forwards_muted.subscribe();
        if muted.wait_for(|muted| !muted).await.is_err() {
            std::future::pending::<()>().await;
        }
    }

    pub(super) fn count(&self, change: impl FnOnce(&mut FakeCounters)) {
        self.counters.send_modify(change);
    }

    /// Counts a new connection as open and gives it its number.
    pub(super) fn admit(self: &Arc<Self>) -> Connection {
        let mut number = 0;
        self.count(|counters| {
            number = counters.connections;
            counters.connections += 1;
            counters.open_connections += 1;
        });
        Connection { number, shared: Arc::clone(self) }
    }

    /// True when the connection of this request must end after its answer: the request
    /// has no credential, or `fault` is the server that refuses its token.
    pub(super) fn ends_after(&self, request: &SeenRequest, fault: Option<&Fault>) -> bool {
        let refused = fault.is_some_and(Fault::refuses_the_credential);
        self.ends_unauthenticated.load(Ordering::Relaxed) && (refused || !request.headers.contains_key(AUTHORIZATION))
    }

    pub(super) fn add_fault(&self, fault: Fault) {
        self.script().faults.push(fault);
    }

    pub(super) fn clear_faults(&self) {
        self.script().faults.clear();
    }

    /// The first fault in the sequence of `add_fault` that takes the request.
    pub(super) fn fault_for(&self, request: &SeenRequest) -> Option<Fault> {
        let now = Instant::now();
        self.script().faults.iter().find(|fault| fault.takes(request, now)).cloned()
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
