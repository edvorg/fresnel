//! The link between the requests of a channel and its connection task.
//!
//! One task connects. Each request that needs a connection waits for the one result of that
//! task: the connection, or the error of the connect.

use std::sync::atomic::{AtomicU64, Ordering};

use bytes::Bytes;
use http_body_util::Full;
use hyper::client::conn::http2::SendRequest;
use tokio::sync::watch;

use crate::error::NetError;

/// Sends requests on one HTTP/2 connection.
pub(crate) type Sender = SendRequest<Full<Bytes>>;

#[derive(Clone)]
enum Link {
    /// No connection. The error is the error of the last connect, if that connect failed.
    Down(Option<NetError>),
    /// A connection, and its number in the sequence of the connections of the channel.
    Up(Sender, u64),
}

pub(crate) struct LinkBoard {
    link: watch::Sender<Link>,
    /// The count of connects that failed. A request reads it at its start, so it can tell
    /// the result of a new connect from the error of an old one.
    failed_connects: AtomicU64,
}

impl LinkBoard {
    pub(crate) fn new() -> Self {
        Self { link: watch::Sender::new(Link::Down(None)), failed_connects: AtomicU64::new(0) }
    }

    pub(crate) fn up(&self, sender: Sender, generation: u64) {
        self.link.send_replace(Link::Up(sender, generation));
    }

    /// The connection ended. A request now waits for the next connect.
    pub(crate) fn lost(&self) {
        self.link.send_replace(Link::Down(None));
    }

    /// A connect failed. Each request that waited for it gets the error.
    pub(crate) fn connect_failed(&self, error: NetError) {
        self.failed_connects.fetch_add(1, Ordering::SeqCst);
        self.link.send_replace(Link::Down(Some(error)));
    }

    /// The sender of the open connection, with the number of that connection. With no
    /// connection, the call waits for the result of the connect in progress or of the next one.
    pub(crate) async fn sender(&self) -> Result<(Sender, u64), NetError> {
        let failed_at_start = self.failed_connects.load(Ordering::SeqCst);
        let mut link = self.link.subscribe();
        loop {
            match &*link.borrow_and_update() {
                Link::Up(sender, generation) if !sender.is_closed() => return Ok((sender.clone(), *generation)),
                Link::Down(Some(error)) if self.failed_connects.load(Ordering::SeqCst) > failed_at_start => {
                    return Err(error.clone());
                }
                // A closed connection that the task did not see yet, or an old error.
                Link::Up(..) | Link::Down(_) => {}
            }
            if link.changed().await.is_err() {
                return Err(NetError::network("the channel stopped"));
            }
        }
    }
}
