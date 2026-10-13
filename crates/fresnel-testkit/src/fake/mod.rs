//! A fake API server for the faults that a real cluster cannot make on order.
//!
//! It works on real sockets with TLS, HTTP/2 and HTTP/1.1, so the app uses its real network
//! code. It is not a Kubernetes server: it knows no kinds and keeps no objects.
//!
//! - `GET /version` gives a version.
//! - A request with `watch=true` gives a stream. The test sends each event line.
//! - An HTTP/1.1 request with the header `Upgrade` gets HTTP 101, and then each byte back.
//! - Each other request gets the body that the test set for its path, or an empty list.
//!
//! A [`Fault`] replaces these answers. For a network fault, put a `FaultProxy` before the server.

mod answer;
mod script;
mod seen;
mod serve;
mod state;
mod tls;
mod watch;

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use fresnel_net::task::TaskHandle;
use tokio::net::TcpListener;

pub use script::Fault;
pub use seen::SeenRequest;

use crate::KitError;
use crate::background::Background;
use state::Shared;
use watch::Feed;

/// The stream limit of a kube-apiserver for one HTTP/2 connection.
const API_SERVER_STREAMS: u32 = 100;

/// A copy of the counters of the fake server.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FakeCounters {
    /// TCP connections that clients opened since the start.
    pub connections: u64,
    /// Connections that are open now.
    pub open_connections: u64,
    /// Connections that ended with an error, for example a reset by the client.
    pub broken: u64,
    /// Requests whose head arrived.
    pub requests: u64,
    /// Requests that got no answer because of a fault.
    pub unanswered: u64,
    /// Watch streams that are open now.
    pub open_watches: u64,
    /// Watch streams that the server ended because they were more than 4096 events behind.
    pub dropped_watches: u64,
    /// Connections that changed to a raw stream after HTTP 101.
    pub upgrades: u64,
}

/// A running fake API server. It stops when this value drops.
pub struct FakeApiServer {
    addr: SocketAddr,
    certificate: Vec<u8>,
    shared: Arc<Shared>,
    background: Background,
    _driver: TaskHandle,
}

impl FakeApiServer {
    /// Listens on a free port of 127.0.0.1, with the stream limit of a kube-apiserver.
    pub async fn start() -> Result<Self, KitError> {
        Self::start_with_stream_limit(API_SERVER_STREAMS).await
    }

    /// The same with an other value of `SETTINGS_MAX_CONCURRENT_STREAMS`.
    pub async fn start_with_stream_limit(stream_limit: u32) -> Result<Self, KitError> {
        let identity = tls::identity()?;
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let addr = listener.local_addr()?;
        let shared = Arc::new(Shared::new(listener, identity.acceptor, stream_limit));
        let background = Background::new()?;
        let driver = background.start("fake API server", {
            let shared = Arc::clone(&shared);
            move || serve::run(Arc::clone(&shared))
        });
        Ok(Self { addr, certificate: identity.certificate, shared, background, _driver: driver })
    }

    /// The address that the client connects to.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The certificate of the server in DER form. It is valid for 127.0.0.1 and localhost.
    /// The client of a test trusts this one certificate.
    pub fn certificate_der(&self) -> &[u8] {
        &self.certificate
    }

    /// Sets the JSON body that each request for `path` gets.
    pub fn set_body(&self, path: &str, json: &str) {
        self.shared.set_body(path, json.to_owned().into());
    }

    /// Sends one event line to each open watch stream of `path`. Returns the count of open
    /// watch streams of all paths. With 0, nobody got the event.
    ///
    /// The server sends the line later, when its task runs. It holds 4096 events for each
    /// stream. A stream that is more behind ends, and the counter `dropped_watches` shows it.
    pub fn send_event(&self, path: &str, json_line: &str) -> usize {
        let line = format!("{json_line}\n").into();
        self.shared.feed.send(Feed::Event { path: path.into(), line }).unwrap_or(0)
    }

    /// Ends each open watch stream with a clean end of data, as the server timeout of a
    /// watch does. Returns the count of streams.
    pub fn end_watches(&self) -> usize {
        self.shared.feed.send(Feed::End).unwrap_or(0)
    }

    /// Adds a fault. A request gets the first fault that takes it, in the sequence of the adds.
    pub fn add_fault(&self, fault: Fault) {
        self.shared.add_fault(fault);
    }

    /// Removes all faults.
    pub fn clear_faults(&self) {
        self.shared.clear_faults();
    }

    /// Sends GOAWAY on each open HTTP/2 connection. Such a connection takes no new stream
    /// and closes after its last open stream ends.
    pub fn go_away(&self) {
        self.shared.go_away.send_modify(|count| *count += 1);
    }

    /// From now on, the server ends an HTTP/2 connection with GOAWAY after its answer to a
    /// request with no credential, and after the HTTP 401 of a fault with no path. A
    /// kube-apiserver does this to each request that does not pass its token check. The
    /// HTTP 401 of a fault on a path ends no connection: it stands for an aggregated API.
    pub fn end_unauthenticated_connections(&self) {
        self.shared.ends_unauthenticated.store(true, Ordering::Relaxed);
    }

    /// The heads of the requests that the server saw, in their sequence.
    pub fn requests(&self) -> Vec<SeenRequest> {
        self.shared.seen()
    }

    /// A copy of the counters.
    pub fn counters(&self) -> FakeCounters {
        *self.shared.counters.borrow()
    }

    /// Waits until the counters meet the condition and gives that copy. A test waits for this
    /// signal and not for a fixed time.
    pub async fn wait_for(&self, mut condition: impl FnMut(&FakeCounters) -> bool) -> FakeCounters {
        let mut counters = self.shared.counters.subscribe();
        let met = counters.wait_for(|counters| condition(counters)).await.map(|counters| *counters);
        met.unwrap_or_else(|_| self.counters())
    }

    /// One line for each run of the server task that ended. An empty list is the normal case.
    pub fn task_failures(&self) -> Vec<String> {
        self.background.failures()
    }
}
