//! One connection of a local client, and its two ends: a clean close, and a close with an
//! error that the client sees.
//!
//! The error is a TCP reset: a close with a linger time of 0. In the port forward prototype,
//! macOS lost such a reset in 74 of 100 transfers when data was on the way to the client
//! (test X5), and the client then waited with no end. A reset with no data on the way always
//! arrived. So [`LocalStream::reset`] first waits until the client took each byte, for 2 s
//! at most: a client that reads nothing gets the reset then. A connection that drops with
//! no close gets the reset at once: a stop of the app or a panic never ends it silently.

use std::net::SocketAddr;
use std::time::Duration;

use socket2::SockRef;
use tokio::io::AsyncWriteExt as _;
use tokio::net::TcpStream;
use tokio::net::tcp::{ReadHalf, WriteHalf};
use tokio::time::{Instant, sleep};

use crate::platform;

/// The longest wait of a reset for the client to take the bytes that it did not take yet.
const DRAIN_LIMIT: Duration = Duration::from_secs(2);
/// The pause between two looks at those bytes.
const DRAIN_STEP: Duration = Duration::from_millis(2);

/// One connection of a local client. A drop with no [`LocalStream::finish`] resets it.
#[derive(Debug)]
pub struct LocalStream {
    /// `None` after a close.
    stream: Option<TcpStream>,
    peer: SocketAddr,
}

impl LocalStream {
    pub(super) fn new(stream: TcpStream, peer: SocketAddr) -> Self {
        // A small write of the client goes out at once: a terminal of a database, for example.
        // An error leaves the default, which only delays.
        match stream.set_nodelay(true) {
            Ok(()) | Err(_) => {}
        }
        Self { stream: Some(stream), peer }
    }

    /// The address of the client.
    pub fn peer(&self) -> SocketAddr {
        self.peer
    }

    /// The two directions, for one task that reads and writes at the same time. `None` after
    /// a close.
    pub fn split(&mut self) -> Option<(ReadHalf<'_>, WriteHalf<'_>)> {
        self.stream.as_mut().map(TcpStream::split)
    }

    /// The clean end: the client reads the end of the data after each byte.
    pub async fn finish(mut self) {
        if let Some(mut stream) = self.stream.take() {
            // An error tells that the client is gone already: nothing waits for the end.
            match stream.shutdown().await {
                Ok(()) | Err(_) => {}
            }
        }
    }

    /// The end with an error for the client: a reset, after the client took the bytes on the
    /// way, for [`DRAIN_LIMIT`] at most.
    pub async fn reset(mut self) {
        if let Some(stream) = self.stream.as_ref() {
            let end = Instant::now() + DRAIN_LIMIT;
            // An error of the system reads as "no bytes": the reset then goes at once.
            while platform::unacknowledged(stream).is_ok_and(|bytes| bytes > 0) && Instant::now() < end {
                sleep(DRAIN_STEP).await;
            }
        }
        self.reset_now();
    }

    fn reset_now(&mut self) {
        if let Some(stream) = self.stream.take() {
            // With no linger time, the close is a FIN. No better signal exists then.
            match SockRef::from(&stream).set_linger(Some(Duration::ZERO)) {
                Ok(()) | Err(_) => {}
            }
            drop(stream);
        }
    }
}

impl Drop for LocalStream {
    fn drop(&mut self) {
        self.reset_now();
    }
}
