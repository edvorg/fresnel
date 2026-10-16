//! The task of one socket: one loop of reads, writes, pings and the close.
//!
//! The two halves of the WebSocket work in the same loop, and no arm waits in its body. A
//! write that waits for the peer thus stops no read, no ping and no close: in the terminal
//! prototype, pings waited behind such a write, and a good connection failed as silent.
//!
//! A ping carries the count of the data bytes before it, and its answer gives the count
//! back: the peer read each byte up to there. A peer that reads nothing, as a pod whose input
//! is full, answers no ping, also when the buffers of the system took the ping. So a ping with
//! no answer behind many such bytes asks the path, as a ping behind a waiting write does.

use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, resume_unwind};
use std::sync::Arc;
use std::time::SystemTime;

use bytes::Bytes;
use futures_util::stream::SplitStream;
use futures_util::{FutureExt as _, SinkExt as _, StreamExt as _};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{Notify, mpsc};
use tokio::time::{Instant, timeout};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};

use super::board::Board;
use super::liveness::{Due, Liveness};
use super::wire::{self, Sink};
use super::{Path, SocketEnd, SocketFailure, SocketParts, SocketSettings};
use crate::changes::Changes;
use crate::error::NetError;
use crate::signal::Listener;
use crate::task::panic_message;

/// Below this count of bytes that the peer did not confirm, a ping with no answer is a silent
/// connection: a process that reads no input fills the pipe of its input first, 64 KiB on
/// Linux, and the server reads on until then. The keys of a terminal are far below it.
const UNREAD: u64 = 64 * 1024;

pub(super) struct Run<S> {
    settings: SocketSettings,
    sink: Sink<S>,
    stream: SplitStream<WebSocketStream<S>>,
    outgoing: mpsc::Receiver<Bytes>,
    delivery: mpsc::Sender<Bytes>,
    close: Arc<Notify>,
    listener: Listener,
    path: Arc<dyn Path>,
    changes: Changes,
    board: Board,
    live: Liveness,
    /// The messages for the sink, in their sequence: one message of the caller at most, one
    /// ping at most and the close.
    queue: VecDeque<Message>,
    /// The sink holds bytes that are not out yet.
    unflushed: bool,
    /// A ping waits in the queue or in the sink. A second one would carry the same count, and
    /// with each wake while a write waits, the queue and the buffer of the sink would grow.
    ping_waits: bool,
    /// Since when bytes wait to go out. `None` while each byte is out.
    blocked: Option<Instant>,
    /// A message of the server that waits for room in the queue of the caller.
    held: Option<Bytes>,
    /// The end of the wait for the answer to the close of the app.
    closing: Option<Instant>,
    /// The caller can send nothing more, and each message that waited went to `queue`.
    drained: bool,
    /// A write failed: the read goes on until this time, so that the last messages of the
    /// server still arrive. The status of an exec comes just before the end of its connection,
    /// and a key that the user typed after `exit` breaks the write first.
    write_failed: Option<(String, Instant)>,
    /// The data bytes that went to the queue, and those that the answer of a ping confirmed.
    sent: u64,
    confirmed: u64,
    /// A ping got no answer, and the live path made it a peer that reads nothing.
    behind: bool,
}

impl<S: AsyncRead + AsyncWrite + Unpin + Send> Run<S> {
    pub(super) fn new(
        socket: WebSocketStream<S>,
        parts: &SocketParts,
        outgoing: mpsc::Receiver<Bytes>,
        delivery: mpsc::Sender<Bytes>,
        close: Arc<Notify>,
        board: Board,
    ) -> Self {
        let (sink, stream) = socket.split();
        let settings = parts.settings.clone();
        let live = Liveness::new(Instant::now(), settings.ping_after, settings.answer_limit);
        Self {
            sink,
            stream,
            outgoing,
            delivery,
            close,
            listener: parts.signal.listen(),
            path: Arc::clone(&parts.path),
            changes: parts.changed.watch(),
            board,
            live,
            queue: VecDeque::new(),
            unflushed: false,
            ping_waits: false,
            blocked: None,
            held: None,
            closing: None,
            drained: false,
            write_failed: None,
            sent: 0,
            confirmed: 0,
            behind: false,
            settings,
        }
    }

    /// Runs the socket to its end, and then waits until the owner drops the socket. The end
    /// of a socket is no end of a task: the state and the report of the socket tell it, and
    /// the journal gets no entry of the start function for each tunnel of a forward. A panic
    /// gives the final state first, and then goes on to the start function, which reports it.
    pub(super) async fn run(mut self) -> Result<(), NetError> {
        let end = match AssertUnwindSafe(self.until_end()).catch_unwind().await {
            Ok(end) => end,
            Err(panic) => {
                self.board.end(SocketEnd::Failed(SocketFailure::Panicked(panic_message(panic.as_ref()))));
                resume_unwind(panic);
            }
        };
        let answers = matches!(end, SocketEnd::ClosedByServer { .. });
        let close_limit = self.settings.close_limit;
        let (mut sink, mut board, held, delivery) = self.into_end();
        board.end(end);
        if answers {
            // The answer to the close of the server waits in the sink. The result tells nothing
            // new: after it, the protocol has ended either way.
            match timeout(close_limit, sink.flush()).await {
                Ok(Ok(())) | Ok(Err(_)) | Err(_) => {}
            }
        }
        drop(sink);
        // The owner reads the last message before the end of its queue.
        if let Some(message) = held
            && let Ok(permit) = delivery.reserve().await
        {
            permit.send(message);
        }
        drop(delivery);
        std::future::pending::<()>().await;
        Ok(())
    }

    /// The parts that outlive the loop. Each other part drops at the return, so each writer
    /// gets an error at once; the connection closes with the drop of the sink.
    fn into_end(self) -> (Sink<S>, Board, Option<Bytes>, mpsc::Sender<Bytes>) {
        let Self { sink, board, held, delivery, .. } = self;
        (sink, board, held, delivery)
    }

    async fn until_end(&mut self) -> SocketEnd {
        loop {
            if let Some(end) = self.step().await {
                return end;
            }
        }
    }

    /// Waits for the next event and handles it. Gives the end, if the event is one.
    async fn step(&mut self) -> Option<SocketEnd> {
        let (deadline, writes, takes) = (self.deadline(), self.writes(), self.takes());
        let end = tokio::select! {
            read = self.stream.next(), if self.held.is_none() => self.on_read(read),
            () = wire::deliver(&self.delivery, &mut self.held), if self.held.is_some() => {
                self.live.resume(Instant::now());
                None
            }
            written = wire::write_out(&mut self.sink, &mut self.queue, &mut self.unflushed), if writes => {
                self.on_written(written);
                None
            }
            message = self.outgoing.recv(), if takes => {
                self.take(message);
                None
            }
            () = wire::at(deadline) => self.on_deadline(),
            () = self.close.notified(), if self.closing.is_none() => {
                self.begin_close();
                None
            }
            _cause = self.listener.raised() => {
                self.on_signal();
                None
            }
            () = self.changes.next(), if self.live.on_path() => self.path_changed(),
        };
        if end.is_none() {
            self.show_waits();
        }
        end
    }

    fn on_read(&mut self, read: Option<Result<Message, WsError>>) -> Option<SocketEnd> {
        let message = match read {
            Some(Ok(message)) => message,
            None => return Some(self.read_end(SocketEnd::EndedByServer)),
            Some(Err(error)) => return Some(self.read_end(wire::read_failed(error))),
        };
        self.live.heard(Instant::now());
        self.behind = false;
        match message {
            Message::Binary(data) => {
                self.board.counts().received(data.len());
                match self.delivery.try_send(data) {
                    // Closed: the owner dropped the socket, and the drop aborts this task.
                    Ok(()) | Err(TrySendError::Closed(_)) => {}
                    Err(TrySendError::Full(data)) => {
                        self.held = Some(data);
                        self.live.pause();
                    }
                }
                None
            }
            Message::Pong(count) => {
                self.confirmed = self.confirmed.max(wire::count(&count).min(self.sent));
                None
            }
            // tungstenite answers a ping of the server by itself.
            Message::Ping(_) | Message::Frame(_) => None,
            Message::Text(_) => {
                let text = "the server sent a text message, and the subprotocol is binary";
                Some(SocketEnd::Failed(SocketFailure::Invalid(text.to_owned())))
            }
            Message::Close(frame) => Some(self.read_end(wire::closed_by_server(frame))),
        }
    }

    /// The end of the read: the close of the app when it asked for one, else what the server did.
    fn read_end(&self, end: SocketEnd) -> SocketEnd {
        if self.closing.is_some() { SocketEnd::ClosedByApp } else { end }
    }

    fn on_written(&mut self, written: Result<(), WsError>) {
        let now = Instant::now();
        match written {
            Ok(()) => {
                self.blocked = None;
                self.ping_waits = false;
                self.live.written(now);
            }
            Err(error) => {
                self.queue.clear();
                self.unflushed = false;
                self.ping_waits = false;
                self.blocked = None;
                self.behind = false;
                // Each writer gets an error at once.
                self.outgoing.close();
                self.write_failed = Some((error.to_string(), now + self.settings.answer_limit));
            }
        }
    }

    fn take(&mut self, message: Option<Bytes>) {
        match message {
            Some(data) => {
                self.board.counts().sent(data.len());
                self.sent = self.sent.saturating_add(u64::try_from(data.len()).unwrap_or(u64::MAX));
                self.push(Message::Binary(data));
            }
            // Each sender is closed, by the close of the app, and each message is out.
            None => {
                self.drained = true;
                if self.closing.is_some() {
                    self.push(wire::close());
                }
            }
        }
    }

    fn push(&mut self, message: Message) {
        self.queue.push_back(message);
        self.blocked.get_or_insert_with(Instant::now);
    }

    fn begin_close(&mut self) {
        self.closing = Some(Instant::now() + self.settings.close_limit);
        // The messages that wait still go out, then the close message.
        self.outgoing.close();
        self.board.closing();
    }

    /// A wake or a network change: a test of the connection with the short limit.
    fn on_signal(&mut self) {
        if self.checks_liveness() && self.live.test(Instant::now(), self.settings.test_limit) {
            self.ping();
        }
    }

    /// Queues a ping with the count of the data bytes before it, unless one waits. That one
    /// has the same count: a message of the caller goes in only after each byte before it is out.
    fn ping(&mut self) {
        if !self.ping_waits {
            self.ping_waits = true;
            self.push(wire::ping(self.sent));
        }
    }

    fn on_deadline(&mut self) -> Option<SocketEnd> {
        let now = Instant::now();
        if self.closing.is_some_and(|until| now >= until) {
            return Some(SocketEnd::ClosedByApp);
        }
        if let Some((text, until)) = &self.write_failed
            && now >= *until
        {
            return Some(SocketEnd::Failed(SocketFailure::Broken(text.clone())));
        }
        if !self.checks_liveness() {
            return None;
        }
        match self.live.due(now)? {
            Due::Ping => {
                self.live.ping_queued(now);
                self.ping();
                None
            }
            Due::Silent if self.sent.saturating_sub(self.confirmed) < UNREAD => Some(self.silent(now)),
            Due::Silent | Due::AskPath if self.path.live_since(self.live.heard_at()) => {
                self.live.path_live(now);
                self.behind = true;
                None
            }
            Due::Silent | Due::AskPath => Some(self.silent(now)),
        }
    }

    /// The state shows a wait while a write waits for its limit, or while the peer is behind.
    fn show_waits(&mut self) {
        let now = Instant::now();
        let blocked =
            self.blocked.map(|since| now.duration_since(since)).filter(|waited| *waited >= self.settings.answer_limit);
        let behind = self.behind.then(|| self.live.quiet(now));
        match blocked.max(behind) {
            Some(waited) if !self.board.shows_waits() => {
                self.board.waits(Some(SystemTime::now().checked_sub(waited).unwrap_or_else(SystemTime::now)));
            }
            None => self.board.waits(None),
            Some(_) => {}
        }
    }

    /// The cluster changed while the peer is behind. With no channel that has been live since
    /// the last frame, the silence of the socket is a break.
    fn path_changed(&mut self) -> Option<SocketEnd> {
        (!self.path.live_since(self.live.heard_at())).then(|| self.silent(Instant::now()))
    }

    fn silent(&self, now: Instant) -> SocketEnd {
        SocketEnd::Failed(SocketFailure::Silent { quiet: self.live.quiet(now) })
    }

    /// The pings stop during a close and after a failed write: each one has its own limit.
    fn checks_liveness(&self) -> bool {
        self.closing.is_none() && self.write_failed.is_none()
    }

    fn writes(&self) -> bool {
        self.write_failed.is_none() && (!self.queue.is_empty() || self.unflushed)
    }

    /// A new message of the caller goes into the sink only after each byte before it is out.
    fn takes(&self) -> bool {
        self.write_failed.is_none() && self.queue.is_empty() && !self.unflushed && !self.drained
    }

    fn deadline(&self) -> Option<Instant> {
        let waits = self.blocked.filter(|_| !self.board.shows_waits());
        let waits = waits.map(|since| since + self.settings.answer_limit);
        let live = if self.checks_liveness() { self.live.deadline() } else { None };
        let failed = self.write_failed.as_ref().map(|(_, until)| *until);
        [self.closing, failed, waits, live].into_iter().flatten().min()
    }
}
