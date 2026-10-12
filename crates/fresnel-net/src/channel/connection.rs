//! One HTTP/2 connection of hyper, run inside the task of its channel.
//!
//! hyper gives the parts of a connection to an executor: the socket loop and one future for
//! each request. This executor hands them to the task of the channel. Thus no task starts
//! outside the start function, and the drop of a connection ends each of its streams.

use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};

use bytes::Bytes;
use futures_util::StreamExt as _;
use futures_util::future::BoxFuture;
use futures_util::stream::FuturesUnordered;
use http_body_util::Full;
use hyper::client::conn::http2;
use hyper_util::rt::{TokioIo, TokioTimer};
use tokio::sync::Notify;

use super::ChannelSettings;
use super::link::Sender;
use crate::connect::dial::Io;
use crate::error::{NetError, chain};

/// The futures that hyper started and the task of the channel did not take yet.
#[derive(Clone, Default)]
pub(super) struct Started {
    futures: Arc<Mutex<Vec<BoxFuture<'static, ()>>>>,
    added: Arc<Notify>,
}

impl<F> hyper::rt::Executor<F> for Started
where
    F: Future<Output = ()> + Send + 'static,
{
    fn execute(&self, future: F) {
        self.futures.lock().unwrap_or_else(PoisonError::into_inner).push(Box::pin(future));
        self.added.notify_one();
    }
}

impl Started {
    fn take(&self) -> Vec<BoxFuture<'static, ()>> {
        std::mem::take(&mut *self.futures.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

/// How the request side of a connection ended.
#[derive(Debug)]
pub(super) enum End {
    /// A clean end: the server closed the connection or sent GOAWAY. After GOAWAY, the open
    /// streams continue until [`Connection::drain`] returns.
    Closed,
    /// An error of the connection. Each of its streams is lost.
    Failed(String),
}

type Dispatcher = http2::Connection<TokioIo<Io>, Full<Bytes>, Started>;

pub(super) struct Connection {
    /// Takes the requests of the senders. `None` after its end, so that a sender sees a
    /// closed connection and gets a request back that was not written.
    dispatcher: Option<Pin<Box<Dispatcher>>>,
    started: Started,
    running: FuturesUnordered<BoxFuture<'static, ()>>,
}

impl Connection {
    /// The HTTP/2 handshake with the keepalive ping and the fixed windows of the settings.
    pub(super) async fn handshake(io: Io, settings: &ChannelSettings) -> Result<(Sender, Self), NetError> {
        let started = Started::default();
        let mut builder = http2::Builder::new(started.clone());
        builder
            .timer(TokioTimer::new())
            .keep_alive_interval(settings.keepalive_interval)
            .keep_alive_timeout(settings.keepalive_timeout)
            // A watch channel with no event is idle for the ping, and it must detect a silent connection.
            .keep_alive_while_idle(true)
            // A window that adapts lets one body that nobody reads stop each other stream.
            .adaptive_window(false)
            .initial_stream_window_size(settings.stream_window)
            .initial_connection_window_size(settings.connection_window);
        let (sender, dispatcher) = builder
            .handshake(TokioIo::new(io))
            .await
            .map_err(|error| NetError::because("the HTTP/2 handshake failed", &error))?;
        let connection = Self { dispatcher: Some(Box::pin(dispatcher)), started, running: FuturesUnordered::new() };
        Ok((sender, connection))
    }

    /// Runs the connection until its request side ends. The future can drop at each await
    /// and start again: the state is in `self`.
    pub(super) async fn run(&mut self) -> End {
        loop {
            let Some(dispatcher) = self.dispatcher.as_mut() else {
                return End::Closed;
            };
            tokio::select! {
                ended = dispatcher => {
                    self.dispatcher = None;
                    return match ended {
                        Ok(()) => End::Closed,
                        Err(error) => End::Failed(chain(&error)),
                    };
                }
                () = self.started.added.notified() => self.running.extend(self.started.take()),
                Some(()) = self.running.next() => {}
            }
        }
    }

    /// Runs the streams that are open after the end of the request side. Returns when the
    /// last one ended.
    pub(super) async fn drain(mut self) {
        self.dispatcher = None;
        loop {
            self.running.extend(self.started.take());
            tokio::select! {
                ended = self.running.next() => if ended.is_none() {
                    return;
                },
                () = self.started.added.notified() => {}
            }
        }
    }

    /// The value of `SETTINGS_MAX_CONCURRENT_STREAMS` of the server. Before the server sent
    /// its settings, this is the start value of hyper.
    pub(super) fn stream_limit(&self) -> Option<usize> {
        self.dispatcher.as_ref().map(|dispatcher| dispatcher.current_max_send_streams())
    }
}
