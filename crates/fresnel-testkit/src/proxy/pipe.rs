//! One direction of one connection: a reader, a queue of chunks and a writer.
//!
//! The reader never waits for the writer of a chunk. Thus latency delays each byte and does
//! not limit the data rate. A proxy that sleeps before each write passes one chunk for each
//! delay, which makes a far cluster look much slower than it is.

use std::future::pending;
use std::io;
use std::time::Duration;

use socket2::SockRef;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{ReadHalf, WriteHalf};
use tokio::sync::{Semaphore, SemaphorePermit, mpsc};
use tokio::time::{Instant, sleep_until};

use super::ProxyMode;
use super::state::Shared;

/// The size of one read.
const CHUNK_BYTES: usize = 64 * 1024;
/// The bytes that one direction holds between its reader and its writer. With latency, this
/// is the data on the way. The reader waits when the queue is full.
const QUEUE_BYTES: usize = 64 * 1024 * 1024;
/// The smallest part of the queue that one chunk takes. A chunk needs memory for more than
/// its bytes, so chunks of one byte would have no useful limit without this.
const CHUNK_COST: usize = 1024;

/// What the faults of the proxy mean for one connection.
#[derive(Clone, Copy)]
pub(super) struct Gate<'a> {
    shared: &'a Shared,
    connection: u64,
}

impl<'a> Gate<'a> {
    pub(super) fn new(shared: &'a Shared, connection: u64) -> Self {
        Self { shared, connection }
    }

    /// Returns when the connection can carry bytes or its close: at once in most modes,
    /// after the end of a freeze, and never for a silent connection.
    pub(super) async fn pass(&self) {
        let mut mode = self.shared.mode.subscribe();
        let thawed = mode.wait_for(|mode| *mode != ProxyMode::Freeze).await.is_ok();
        if !thawed || self.shared.is_silent(self.connection) {
            pending::<()>().await;
        }
    }

    fn latency(&self) -> Duration {
        let mode = *self.shared.mode.borrow();
        match mode {
            ProxyMode::Latency(one_way) => one_way,
            ProxyMode::Pass | ProxyMode::HoldNew { .. } | ProxyMode::Freeze | ProxyMode::ResetEach(_) => Duration::ZERO,
        }
    }
}

/// The part of the queue that a chunk of `count` bytes takes.
fn queue_cost(count: usize) -> u32 {
    u32::try_from(count.max(CHUNK_COST)).unwrap_or(u32::MAX)
}

/// Bytes on the way to the writer. The permit gives their part of the queue back at the drop.
struct Chunk<'a> {
    due: Instant,
    bytes: Vec<u8>,
    _space: SemaphorePermit<'a>,
}

/// Carries the bytes and then the end of data from one socket to the other.
/// Returns at the end of data, or with the first error of a socket.
pub(super) async fn pipe(mut from: ReadHalf<'_>, mut to: WriteHalf<'_>, gate: Gate<'_>) -> io::Result<()> {
    let space = Semaphore::new(QUEUE_BYTES);
    // The channel has no limit of its own. `space` limits the bytes and the count of chunks.
    let (queue, mut on_the_way) = mpsc::unbounded_channel::<Chunk<'_>>();
    let read = async {
        // The reader owns the sender. Its end closes the queue, and the writer sends the end of data.
        let queue = queue;
        let mut buffer = vec![0_u8; CHUNK_BYTES];
        loop {
            let count = match from.read(&mut buffer).await {
                Ok(count @ 1..) => count,
                end => {
                    // A frozen or silent connection does not carry its end.
                    gate.pass().await;
                    return end.map(|_| ());
                }
            };
            let due = Instant::now() + gate.latency();
            let Ok(permit) = space.acquire_many(queue_cost(count)).await else {
                return Ok(());
            };
            let bytes = buffer[..count].to_vec();
            if queue.send(Chunk { due, bytes, _space: permit }).is_err() {
                return Ok(());
            }
        }
    };
    let write = async {
        while let Some(chunk) = on_the_way.recv().await {
            sleep_until(chunk.due).await;
            gate.pass().await;
            to.write_all(&chunk.bytes).await?;
        }
        // The socket sends its end of data. From now on its close is a normal one.
        SockRef::from(to.as_ref()).set_linger(None)?;
        to.shutdown().await
    };
    tokio::try_join!(read, write).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_small_chunk_takes_the_smallest_cost_and_a_full_chunk_its_bytes() {
        assert_eq!((queue_cost(1), queue_cost(CHUNK_BYTES)), (1024, 65_536));
    }
}
