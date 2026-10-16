//! The loop of a socket on a pipe in memory, with the WebSocket peer of tungstenite as the
//! server, and paused time: each limit of the loop gets its exact time.
//!
//! - `flow`: messages, the queues and a write that waits. `silence`: the ping and its limits.
//! - `end`: each end of a socket, with its state, its counters and its report.

#![allow(clippy::expect_used, reason = "a test stops with a panic")]

mod end;
mod flow;
mod silence;

use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::Bytes;
use futures_util::{SinkExt as _, StreamExt as _};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt as _, DuplexStream, ReadBuf};
use tokio::runtime::Handle;
use tokio::time::{Instant, timeout};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::Role;

use super::{
    Counts, Path, Socket, SocketCounts, SocketEnd, SocketFailure, SocketParts, SocketSettings, SocketSpec, SocketState,
};
use crate::changes::Changed;
use crate::signal::Signal;
use crate::task::{TaskReport, Tasks};

const SECOND: Duration = Duration::from_secs(1);
/// The bytes that the pipe holds in each direction: the send buffer of a socket.
const PIPE: usize = 64 * 1024;

/// The path of the test: live, not live, or a defect of the app, which panics.
struct TestPath(AtomicU8);

const LIVE: u8 = 0;
const DOWN: u8 = 1;
const PANICS: u8 = 2;

impl Path for TestPath {
    fn live(&self) -> bool {
        let path = self.0.load(Ordering::Relaxed);
        assert_ne!(path, PANICS, "a defect in the path");
        path == LIVE
    }
}

/// The pipe of the socket. After `fail_writes`, each write of the socket fails as with a
/// reset of its peer, and its reads go on.
struct Pipe {
    inner: DuplexStream,
    writes_fail: Arc<AtomicBool>,
}

impl AsyncRead for Pipe {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for Pipe {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
        if self.writes_fail.load(Ordering::Relaxed) {
            return Poll::Ready(Err(std::io::ErrorKind::BrokenPipe.into()));
        }
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// One socket, its peer and what the test reads of it. The peer acts only when the test
/// drives it: a peer that the test does not read is silent.
struct Bed {
    socket: Socket,
    peer: WebSocketStream<DuplexStream>,
    path: Arc<TestPath>,
    signal: Signal,
    changed: Changed,
    counts: Arc<Counts>,
    writes_fail: Arc<AtomicBool>,
    ends: Arc<Mutex<Vec<SocketEnd>>>,
    reports: Arc<Mutex<Vec<String>>>,
}

impl Bed {
    async fn start() -> Self {
        Self::with(SocketSettings::default()).await
    }

    async fn with(settings: SocketSettings) -> Self {
        Self::with_pipe(settings, PIPE).await
    }

    /// A socket whose pipe holds `pipe` bytes in each direction.
    async fn with_pipe(settings: SocketSettings, pipe: usize) -> Self {
        let (client, server) = tokio::io::duplex(pipe);
        let reports = Arc::new(Mutex::new(Vec::new()));
        let kept = Arc::clone(&reports);
        let tasks = Tasks::new(Handle::current(), move |report: TaskReport| {
            kept.lock().expect("a lock").push(report.to_string());
        });
        let (signal, changed, path) = (Signal::new(), Changed::new(), Arc::new(TestPath(AtomicU8::new(LIVE))));
        let counts = Arc::new(Counts::new(changed.clone()));
        let parts = SocketParts {
            tasks,
            cluster: "memory".into(),
            settings,
            signal: signal.clone(),
            path: Arc::clone(&path) as Arc<dyn Path>,
            changed: changed.clone(),
            counts: Arc::clone(&counts),
        };
        let ends = Arc::new(Mutex::new(Vec::new()));
        let reported = Arc::clone(&ends);
        let on_end = Box::new(move |end: &SocketEnd| reported.lock().expect("a lock").push(end.clone()));
        let spec = SocketSpec { role: "test socket", protocols: &["v5.channel.k8s.io"], on_end: Some(on_end) };
        let writes_fail = Arc::new(AtomicBool::new(false));
        let pipe = Pipe { inner: client, writes_fail: Arc::clone(&writes_fail) };
        let socket = Socket::start(pipe, "v5.channel.k8s.io", &parts, spec).await;
        let peer = WebSocketStream::from_raw_socket(server, Role::Server, None).await;
        Self { socket, peer, path, signal, changed, counts, writes_fail, ends, reports }
    }

    fn set_path(&self, path: u8) {
        self.path.0.store(path, Ordering::Relaxed);
        self.changed.notify();
    }

    fn counts(&self) -> SocketCounts {
        self.counts.read()
    }

    fn ends(&self) -> Vec<SocketEnd> {
        self.ends.lock().expect("a lock").clone()
    }

    /// Waits for a state that meets the condition, with paused time. Gives that state.
    async fn until(&self, what: &str, met: impl Fn(&SocketState) -> bool) -> SocketState {
        let mut states = self.socket.states();
        let state = timeout(600 * SECOND, states.wait_for(|state| met(state))).await;
        state.unwrap_or_else(|_| panic!("in time: {what}")).expect("the socket").clone()
    }

    /// Waits for the end of the socket, and gives it with the time from `start`.
    async fn end(&self, start: Instant) -> (SocketEnd, Duration) {
        match self.until("the end", |state| matches!(state, SocketState::Ended(_))).await {
            SocketState::Ended(end) => (end, start.elapsed()),
            other => panic!("not an end: {other:?}"),
        }
    }

    /// The next message of the peer that is not a ping. The peer answers each ping then.
    async fn peer_reads(&mut self) -> Message {
        loop {
            match timeout(600 * SECOND, self.peer.next()).await.expect("a message in time") {
                Some(Ok(Message::Ping(_))) => {}
                Some(Ok(message)) => return message,
                other => panic!("the peer got no message: {other:?}"),
            }
        }
    }

    /// The peer ends its side of the connection with no close message.
    async fn peer_ends(&mut self) {
        self.peer.get_mut().shutdown().await.expect("the end of the pipe");
    }

    async fn peer_sends(&mut self, data: &'static [u8]) {
        self.peer.send(Message::Binary(Bytes::from_static(data))).await.expect("the peer sends");
    }
}
