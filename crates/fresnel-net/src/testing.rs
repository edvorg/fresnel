//! An HTTP/2 server in memory, for tests with paused time.
//!
//! A test with real sockets must use real time. This server works on pipes in memory, so a
//! test of a time limit runs in milliseconds and gives the exact time.

use std::collections::VecDeque;
use std::convert::Infallible;
use std::io::Write as _;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use flate2::Compression;
use flate2::write::GzEncoder;
use futures_util::FutureExt as _;
use futures_util::future::BoxFuture;
use futures_util::stream;
use http::header::{ACCEPT_ENCODING, CONTENT_ENCODING, RETRY_AFTER};
use http::{HeaderMap, HeaderValue, Request, Response, StatusCode};
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{BodyExt as _, Full, StreamBody};
use hyper::body::{Frame, Incoming};
use hyper::server::conn::http2;
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _, DuplexStream, ReadHalf, WriteHalf};
use tokio::runtime::Handle;
use tokio::sync::{broadcast, watch};
use tokio::task::JoinSet;

use crate::changes::Changed;
use crate::channel::Parts;
use crate::connect::dial::{Io, Protocol};
use crate::connect::{ConnectStatus, Dial};
use crate::credential::{Credential, PluginError, TokenSource};
use crate::error::NetError;
use crate::signal::Signal;
use crate::task::Tasks;

const PIPE_BYTES: usize = 256 * 1024;

/// What a connect to the server does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Connects {
    Work,
    /// Each connect fails at once.
    Refused,
    /// Each connect waits with no end.
    Held,
}

/// One request that the server saw.
#[derive(Clone, Debug)]
pub(crate) struct Seen {
    /// The number of the connection in the sequence of connects, from 0.
    pub(crate) connection: u64,
    pub(crate) path: String,
    pub(crate) headers: HeaderMap,
}

/// The answers that a test set in place of the normal ones.
#[derive(Default)]
struct Script {
    /// Each request with this start of its path gets HTTP 401.
    refused_path: Option<&'static str>,
    /// The next requests get HTTP 429 with these values of `Retry-After`.
    throttles: VecDeque<&'static str>,
}

/// A token source with a fixed list of tokens. The last one repeats. The token `panic`
/// stands for a defect in the credential task.
pub(crate) struct Tokens {
    tokens: Mutex<VecDeque<&'static str>>,
}

impl Tokens {
    pub(crate) fn new(tokens: &[&'static str]) -> Arc<Self> {
        Arc::new(Self { tokens: Mutex::new(tokens.iter().copied().collect()) })
    }
}

impl TokenSource for Tokens {
    fn run(&self, _limit: Duration) -> BoxFuture<'_, Result<Credential, PluginError>> {
        let token = {
            let mut tokens = self.tokens.lock().expect("a lock");
            if tokens.len() > 1 { tokens.pop_front() } else { tokens.front().copied() }.expect("a token")
        };
        assert_ne!(token, "panic", "the token source of the test fails");
        let header = HeaderValue::from_str(&format!("Bearer {token}")).expect("a header");
        std::future::ready(Ok(Credential { header, expires_at: None })).boxed()
    }
}

/// The text of the path `/big`: large, so that gzip makes it much smaller.
pub(crate) fn big_text() -> String {
    (0..4000).map(|item| format!(r#"{{"metadata":{{"name":"pod-{item}","namespace":"default"}}}},"#)).collect()
}

pub(crate) struct MemoryServer {
    connects: watch::Sender<Connects>,
    connections: Mutex<JoinSet<()>>,
    shared: Arc<Shared>,
}

/// What the tasks of the connections read.
struct Shared {
    stream_limit: AtomicU32,
    dials: AtomicU64,
    /// Each connection with a smaller number is silent: it carries no more bytes.
    silent_below: AtomicU64,
    /// With `true`, no request gets an answer.
    mute: AtomicBool,
    go_away: watch::Sender<u64>,
    events: broadcast::Sender<Bytes>,
    seen: Mutex<Vec<Seen>>,
    script: Mutex<Script>,
}

impl MemoryServer {
    pub(crate) fn new() -> Arc<Self> {
        let shared = Shared {
            stream_limit: AtomicU32::new(100),
            dials: AtomicU64::new(0),
            silent_below: AtomicU64::new(0),
            mute: AtomicBool::new(false),
            go_away: watch::Sender::new(0),
            events: broadcast::Sender::new(64),
            seen: Mutex::new(Vec::new()),
            script: Mutex::new(Script::default()),
        };
        Arc::new(Self {
            connects: watch::Sender::new(Connects::Work),
            connections: Mutex::new(JoinSet::new()),
            shared: Arc::new(shared),
        })
    }

    /// The parts of a channel that connects to this server.
    pub(crate) fn parts(self: &Arc<Self>) -> Parts {
        Parts {
            tasks: Tasks::new(Handle::current(), |_| {}),
            cluster: "memory".into(),
            dial: Arc::clone(self) as _,
            light_request: Arc::new(|_| {
                Request::get("https://memory/version").body(Full::default()).expect("a request")
            }),
            signal: Signal::new(),
            changed: Changed::new(),
        }
    }

    pub(crate) fn set_stream_limit(&self, limit: u32) {
        self.shared.stream_limit.store(limit, Ordering::SeqCst);
    }

    pub(crate) fn set_connects(&self, connects: Connects) {
        self.connects.send_replace(connects);
    }

    pub(crate) fn set_mute(&self, mute: bool) {
        self.shared.mute.store(mute, Ordering::SeqCst);
    }

    /// The count of connects that gave a connection.
    pub(crate) fn dials(&self) -> u64 {
        self.shared.dials.load(Ordering::SeqCst)
    }

    pub(crate) fn seen(&self) -> Vec<Seen> {
        self.shared.seen.lock().expect("a lock").clone()
    }

    /// Makes each open connection silent for all time. A new connection works.
    pub(crate) fn silence_open(&self) {
        self.shared.silent_below.store(self.dials(), Ordering::SeqCst);
    }

    /// Closes each open connection, as a reset does.
    pub(crate) fn kill_open(&self) {
        self.connections.lock().expect("a lock").abort_all();
    }

    /// Sends GOAWAY on each open connection.
    pub(crate) fn go_away(&self) {
        self.shared.go_away.send_modify(|count| *count += 1);
    }

    /// Each request whose path starts with `prefix` gets HTTP 401.
    pub(crate) fn refuse_path(&self, prefix: &'static str) {
        self.shared.script.lock().expect("a lock").refused_path = Some(prefix);
    }

    /// The next requests get HTTP 429 with these values of `Retry-After`. A light request
    /// does not count.
    pub(crate) fn throttle_next(&self, retry_after: &[&'static str]) {
        self.shared.script.lock().expect("a lock").throttles.extend(retry_after);
    }

    /// Sends one event to each open stream of a path that starts with `/watch`.
    pub(crate) fn send_event(&self, event: &'static str) {
        self.shared.events.send(Bytes::from_static(event.as_bytes())).expect("an open watch stream");
    }

    /// The server speaks HTTP/2 only. A connect for HTTP/1.1 fails.
    async fn connect(&self, protocol: Protocol) -> Result<Io, NetError> {
        let mut connects = self.connects.subscribe();
        let allowed = connects.wait_for(|connects| *connects != Connects::Held).await.map(|connects| *connects);
        if !matches!(allowed, Ok(Connects::Work)) || protocol != Protocol::Http2 {
            return Err(NetError::network("connection refused"));
        }
        let number = self.shared.dials.fetch_add(1, Ordering::SeqCst);
        let (client, client_side) = tokio::io::duplex(PIPE_BYTES);
        let (server_side, server) = tokio::io::duplex(PIPE_BYTES);
        let (from_client, to_client) = tokio::io::split(client_side);
        let (from_server, to_server) = tokio::io::split(server_side);
        let shared = || Arc::clone(&self.shared);
        let mut connections = self.connections.lock().expect("a lock");
        connections.spawn(shared().relay(number, from_client, to_server));
        connections.spawn(shared().relay(number, from_server, to_client));
        connections.spawn(shared().serve(number, server));
        Ok(Box::new(client))
    }
}

impl Shared {
    /// Carries the bytes of one direction. A silent connection keeps its pipes open and
    /// carries nothing, as a path through a NAT device that lost its table.
    async fn relay(self: Arc<Self>, number: u64, mut from: ReadHalf<DuplexStream>, mut to: WriteHalf<DuplexStream>) {
        let mut buffer = vec![0; 16 * 1024];
        loop {
            let count = match from.read(&mut buffer).await {
                Ok(count) if count > 0 => count,
                Ok(_) | Err(_) => break,
            };
            if number < self.silent_below.load(Ordering::SeqCst) {
                std::future::pending::<()>().await;
            }
            if to.write_all(&buffer[..count]).await.is_err() {
                break;
            }
        }
    }

    async fn serve(self: Arc<Self>, number: u64, io: DuplexStream) {
        let service = service_fn(|request: Request<Incoming>| {
            let server = Arc::clone(&self);
            async move { Ok::<_, Infallible>(server.answer(number, &request).await) }
        });
        let mut builder = http2::Builder::new(TokioExecutor::new());
        builder.max_concurrent_streams(self.stream_limit.load(Ordering::SeqCst));
        let connection = builder.serve_connection(TokioIo::new(io), service);
        tokio::pin!(connection);
        let mut go_away = self.go_away.subscribe();
        loop {
            tokio::select! {
                _ended = &mut connection => return,
                Ok(()) = go_away.changed() => connection.as_mut().graceful_shutdown(),
            }
        }
    }

    async fn answer(&self, connection: u64, request: &Request<Incoming>) -> Response<UnsyncBoxBody<Bytes, Infallible>> {
        let path = request.uri().path().to_owned();
        let headers = request.headers().clone();
        self.seen.lock().expect("a lock").push(Seen { connection, path: path.clone(), headers });
        if self.mute.load(Ordering::SeqCst) {
            std::future::pending::<()>().await;
        }
        if let Some(scripted) = self.scripted(&path) {
            return scripted;
        }
        if path.starts_with("/watch") {
            let events = stream::unfold(self.events.subscribe(), |mut events| async move {
                let event = events.recv().await.ok()?;
                Some((Ok(Frame::data(event)), events))
            });
            return Response::new(StreamBody::new(events).boxed_unsync());
        }
        if path != "/big" {
            return text(StatusCode::OK, "ok");
        }
        if request.headers().get(ACCEPT_ENCODING) != Some(&HeaderValue::from_static("gzip")) {
            return text(StatusCode::OK, &big_text());
        }
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(big_text().as_bytes()).expect("a write to memory");
        let mut packed =
            Response::new(Full::new(Bytes::from(encoder.finish().expect("a write to memory"))).boxed_unsync());
        packed.headers_mut().insert(CONTENT_ENCODING, HeaderValue::from_static("gzip"));
        packed
    }

    /// The answer that the script of the test sets for this request.
    fn scripted(&self, path: &str) -> Option<Response<UnsyncBoxBody<Bytes, Infallible>>> {
        let mut script = self.script.lock().expect("a lock");
        if script.refused_path.is_some_and(|prefix| path.starts_with(prefix)) {
            return Some(text(StatusCode::UNAUTHORIZED, "Unauthorized"));
        }
        let retry_after = script.throttles.pop_front().filter(|_| path != "/version")?;
        let mut throttled = text(StatusCode::TOO_MANY_REQUESTS, "Too many requests");
        throttled.headers_mut().insert(RETRY_AFTER, HeaderValue::from_static(retry_after));
        Some(throttled)
    }
}

fn text(status: StatusCode, text: &str) -> Response<UnsyncBoxBody<Bytes, Infallible>> {
    let mut response = Response::new(Full::new(Bytes::from(text.to_owned())).boxed_unsync());
    *response.status_mut() = status;
    response
}

impl Dial for MemoryServer {
    fn dial(&self, protocol: Protocol) -> BoxFuture<'_, Result<Io, NetError>> {
        self.connect(protocol).boxed()
    }

    fn status(&self) -> ConnectStatus {
        ConnectStatus {
            addresses: Vec::new(),
            resolve_error: None,
            connect_time: None,
            hedge_delay: Duration::ZERO,
            connects: self.dials(),
            attempts: self.dials(),
        }
    }
}
