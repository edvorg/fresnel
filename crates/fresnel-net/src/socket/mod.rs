//! The WebSocket of an upgraded connection: exec, attach and port forward (plan sections 8.9,
//! 12.3 and 12.8).
//!
//! The socket of kube-rs sends a ping each 60 s, reads no answer and has no limit for a read,
//! so a silent connection stalls with no end. A socket of fresnel runs a loop of its own: a
//! ping after a time with no frame, and the end when nothing answers it in its limit. A write
//! that waits for a slow peer is not a break while the cluster answers on its channels.
//!
//! - `handshake`: the headers of the request and the check of the answer.
//! - `liveness`: when a ping goes out, and when a silence is a break.
//! - `run`: the task of one socket. `wire`: its writes, its messages and the ends of a read.
//! - `board`: its state, its counters and its end.

mod board;
mod handshake;
mod liveness;
mod run;
#[cfg(test)]
mod tests;
mod wire;

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use bytes::Bytes;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{Notify, mpsc, watch};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::protocol::{Role, WebSocketConfig};

pub(crate) use board::Counts;
pub use board::SocketCounts;
pub use handshake::Offer;

use crate::changes::Changed;
use crate::error::NetError;
use crate::signal::Signal;
use crate::task::{Restart, TaskHandle, TaskName, Tasks};
use board::Board;
use run::Run;

/// The times and sizes of the sockets of one cluster. Each number is a setting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SocketSettings {
    /// A ping goes out when no frame arrived for this time (plan section 16: 10 s).
    pub ping_after: Duration,
    /// The socket breaks when no frame answers a ping in this time (plan section 16: 5 s).
    pub answer_limit: Duration,
    /// The limit of the ping after a wake or a network change, as for the test of a channel.
    pub test_limit: Duration,
    /// The longest wait for the answer of the server to a close of the app.
    pub close_limit: Duration,
    /// The messages that wait in each direction. A full queue stops the writer, or the read
    /// of the socket, so a slow side slows the other one and loses nothing.
    pub queue: usize,
    /// The largest message of the server. The API server sends pieces of 32 KiB at most.
    pub max_message: usize,
}

impl Default for SocketSettings {
    fn default() -> Self {
        Self {
            ping_after: Duration::from_secs(10),
            answer_limit: Duration::from_secs(5),
            test_limit: Duration::from_secs(3),
            close_limit: Duration::from_secs(5),
            queue: 8,
            max_message: 1024 * 1024,
        }
    }
}

impl SocketSettings {
    /// The longest time from the last frame of a silent connection to its break: the time
    /// to the ping and its limit. The acceptance of M5 and M10 asks for 20 s at most.
    pub fn longest_silence(&self) -> Duration {
        self.ping_after + self.answer_limit
    }
}

/// The state of one socket.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SocketState {
    /// The connection carries frames in each direction.
    Open,
    /// A write waits since this time: the peer takes no bytes, for example a pod that reads
    /// no input. The cluster answers on its channels, so this is not a break (plan 12.3).
    WriteWaits {
        /// The wall clock time of the start of the wait.
        since: SystemTime,
    },
    /// The app asked for a close and waits for the answer of the server.
    Closing,
    /// The final state.
    Ended(SocketEnd),
}

/// How a socket ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SocketEnd {
    /// The app closed the socket, and the server answered or its limit ended.
    ClosedByApp,
    /// The owner dropped the socket while it was open. The connection closed with no close message.
    Dropped,
    /// The server sent a close message. 1000 is a normal close.
    ClosedByServer {
        /// The code of the close message, or 1005 for a message with no code.
        code: u16,
        /// The reason of the close message, often empty.
        reason: String,
    },
    /// The server ended the connection with no close message. The API server does this after
    /// the status of an exec and at the end of the connection of a port forward.
    EndedByServer,
    /// An abnormal end.
    Failed(SocketFailure),
}

/// Why a socket broke.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SocketFailure {
    /// No frame arrived for this time, also no answer to a ping.
    Silent {
        /// The time since the last frame.
        quiet: Duration,
    },
    /// A read or a write of the connection failed, for example with a reset.
    Broken(String),
    /// The server broke the WebSocket protocol, or sent a message above the size limit.
    Invalid(String),
    /// The task of the socket panicked.
    Panicked(String),
}

impl SocketEnd {
    /// True for each end that is no fault: a close, the end of the server or of the owner.
    pub fn is_clean(&self) -> bool {
        match self {
            Self::ClosedByApp | Self::Dropped | Self::EndedByServer => true,
            // 1000 is a normal close, 1001 a server that goes away, 1005 a close with no code.
            Self::ClosedByServer { code, .. } => matches!(code, 1000 | 1001 | 1005),
            Self::Failed(_) => false,
        }
    }
}

impl std::fmt::Display for SocketEnd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ClosedByApp => f.write_str("closed by the app"),
            Self::Dropped => f.write_str("dropped by the app with no close message"),
            Self::ClosedByServer { code, reason } if reason.is_empty() => write!(f, "closed by the server ({code})"),
            Self::ClosedByServer { code, reason } => write!(f, "closed by the server ({code}): {reason}"),
            Self::EndedByServer => f.write_str("ended by the server"),
            Self::Failed(SocketFailure::Silent { quiet }) => {
                write!(f, "silent: no frame for {:.1} s, also no answer to a ping", quiet.as_secs_f64())
            }
            Self::Failed(SocketFailure::Broken(text)) => write!(f, "broken: {text}"),
            Self::Failed(SocketFailure::Invalid(text)) => write!(f, "invalid: {text}"),
            Self::Failed(SocketFailure::Panicked(text)) => write!(f, "its task panicked: {text}"),
        }
    }
}

/// Takes the end of a socket one time, for example for the journal. It runs in the task of
/// the socket, also at a panic or a drop, so it must return fast and take no lock for long.
pub type OnEnd = Box<dyn FnOnce(&SocketEnd) + Send>;

/// What the opener of a socket gives.
pub struct SocketSpec {
    /// The role of the socket, for the name of its task, for example "exec socket".
    pub role: &'static str,
    /// The subprotocols that the request offers, the preferred one first.
    pub protocols: &'static [&'static str],
    /// Takes the end of the socket.
    pub on_end: Option<OnEnd>,
}

impl std::fmt::Debug for SocketSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SocketSpec").field("role", &self.role).field("protocols", &self.protocols).finish()
    }
}

/// Tells a socket whether its cluster answers: then a write that waits is not a break.
pub(crate) trait Path: Send + Sync + 'static {
    /// True while a channel of the cluster is live.
    fn live(&self) -> bool;
}

/// What the sockets of one cluster share.
#[derive(Clone)]
pub(crate) struct SocketParts {
    pub(crate) tasks: Tasks,
    /// The name of the cluster, for the names of the tasks.
    pub(crate) cluster: Arc<str>,
    pub(crate) settings: SocketSettings,
    /// A wake or a network change: each socket tests its connection with a ping.
    pub(crate) signal: Signal,
    pub(crate) path: Arc<dyn Path>,
    /// Tells the readers of the cluster that a state changed, and a socket that the path may have.
    pub(crate) changed: Changed,
    pub(crate) counts: Arc<Counts>,
}

/// One open WebSocket. Its task stops, and its connection closes, when this value drops.
pub struct Socket {
    protocol: &'static str,
    writer: Writer,
    incoming: mpsc::Receiver<Bytes>,
    state: watch::Receiver<SocketState>,
    close: Arc<Notify>,
    _task: TaskHandle,
}

/// Sends the messages of one socket. A clone sends to the same socket.
#[derive(Clone, Debug)]
pub struct Writer {
    queue: mpsc::Sender<Bytes>,
}

impl Writer {
    /// Sends one binary message. It waits while the queue is full: a peer that takes no
    /// bytes slows the writer, and no byte is lost. An error after the end of the socket.
    pub async fn send(&self, message: Bytes) -> Result<(), NetError> {
        self.queue.send(message).await.map_err(|_| NetError::network("the WebSocket ended"))
    }
}

impl Socket {
    /// Starts the task of a socket on an upgraded connection.
    pub(crate) async fn start<S>(io: S, protocol: &'static str, parts: &SocketParts, spec: SocketSpec) -> Self
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let settings = &parts.settings;
        let config = WebSocketConfig::default()
            .max_message_size(Some(settings.max_message))
            .max_frame_size(Some(settings.max_message));
        let socket = WebSocketStream::from_raw_socket(io, Role::Client, Some(config)).await;
        let (writer, outgoing) = mpsc::channel(settings.queue.max(1));
        let (delivery, incoming) = mpsc::channel(settings.queue.max(1));
        let (shown, state) = watch::channel(SocketState::Open);
        let close = Arc::new(Notify::new());
        let board = Board::new(shown, Arc::clone(&parts.counts), spec.on_end);
        let mut run = Some(Run::new(socket, parts, outgoing, delivery, Arc::clone(&close), board));
        let name = TaskName::scoped(spec.role, Arc::clone(&parts.cluster));
        let task = parts.tasks.start(name, Restart::Never, move || {
            let run = run.take();
            async move {
                match run {
                    Some(run) => run.run().await,
                    None => Err(NetError::invalid("the task of a socket ran a second time")),
                }
            }
        });
        Self { protocol, writer: Writer { queue: writer }, incoming, state, close, _task: task }
    }

    /// The subprotocol that the server selected.
    pub fn protocol(&self) -> &'static str {
        self.protocol
    }

    /// A handle that sends messages.
    pub fn writer(&self) -> Writer {
        self.writer.clone()
    }

    /// The next message of the server. `None` after the end, which the state then tells.
    pub async fn recv(&mut self) -> Option<Bytes> {
        self.incoming.recv().await
    }

    /// The state now.
    pub fn state(&self) -> SocketState {
        self.state.borrow().clone()
    }

    /// A receiver of each state change, for a caller that waits for one.
    pub fn states(&self) -> watch::Receiver<SocketState> {
        self.state.clone()
    }

    /// Asks for a clean close: the messages that wait go out, then a close message. The
    /// state is `Closing` until the answer of the server, for `close_limit` at most.
    pub fn close(&self) {
        self.close.notify_one();
    }
}

impl std::fmt::Debug for Socket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Socket").field("protocol", &self.protocol).field("state", &self.state()).finish_non_exhaustive()
    }
}
