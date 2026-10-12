//! The open streams of one channel, and its stream budget (plan section 8.6).
//!
//! The server limits the streams of one HTTP/2 connection. hyper holds a request above the
//! limit with no error and no state. The budget keeps the channel below the limit.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use tokio::sync::Notify;
use tokio::time::Instant;

/// The count of open streams of a channel that takes no more streams.
const RETIRED: usize = usize::MAX;
const NEVER: u64 = u64::MAX;

/// The stream counter of one channel.
pub(crate) struct Streams {
    open: AtomicUsize,
    budget: AtomicUsize,
    /// 0 until the server sent its limit.
    server_limit: AtomicUsize,
    /// Wakes each request of the group that waits for a free stream.
    freed: Arc<Notify>,
    started: Instant,
    /// The time of the last frame of a response, in milliseconds from `started`.
    last_frame_ms: AtomicU64,
}

impl Streams {
    /// `budget` is the budget before the server sent its limit.
    pub(crate) fn new(budget: usize, freed: Arc<Notify>) -> Self {
        Self {
            open: AtomicUsize::new(0),
            budget: AtomicUsize::new(budget),
            server_limit: AtomicUsize::new(0),
            freed,
            started: Instant::now(),
            last_frame_ms: AtomicU64::new(NEVER),
        }
    }

    /// Takes one stream if the channel is below its budget.
    pub(crate) fn try_open(self: &Arc<Self>) -> Option<StreamPermit> {
        let budget = self.budget.load(Ordering::Relaxed);
        // A retired channel has the largest count, so it is never below a budget.
        let take = |open: usize| if open < budget { open.checked_add(1) } else { None };
        self.open.fetch_update(Ordering::AcqRel, Ordering::Acquire, take).ok()?;
        Some(StreamPermit(Arc::clone(self)))
    }

    /// Closes the channel for new streams if it has none. A request that takes a stream at
    /// the same moment wins or loses as a whole, so no stream starts on a closed channel.
    pub(crate) fn retire(&self) -> bool {
        self.open.compare_exchange(0, RETIRED, Ordering::AcqRel, Ordering::Acquire).is_ok()
    }

    pub(crate) fn is_retired(&self) -> bool {
        self.open.load(Ordering::Acquire) == RETIRED
    }

    /// The server sent its limit, and `budget` is the part of it that the channel uses.
    pub(crate) fn set_limit(&self, server_limit: usize, budget: usize) {
        self.server_limit.store(server_limit, Ordering::Relaxed);
        if self.budget.swap(budget, Ordering::Relaxed) < budget {
            self.freed.notify_waiters();
        }
    }

    pub(crate) fn open(&self) -> usize {
        match self.open.load(Ordering::Acquire) {
            RETIRED => 0,
            open => open,
        }
    }

    pub(crate) fn budget(&self) -> usize {
        self.budget.load(Ordering::Relaxed)
    }

    pub(crate) fn server_limit(&self) -> Option<usize> {
        Some(self.server_limit.load(Ordering::Relaxed)).filter(|limit| *limit > 0)
    }

    /// A frame of a response arrived now.
    pub(crate) fn frame_arrived(&self) {
        let now = u64::try_from(self.started.elapsed().as_millis()).unwrap_or(NEVER - 1);
        self.last_frame_ms.store(now, Ordering::Relaxed);
    }

    /// The time since the last frame of a response.
    pub(crate) fn last_frame_age(&self) -> Option<Duration> {
        let at = Some(self.last_frame_ms.load(Ordering::Relaxed)).filter(|at| *at != NEVER)?;
        Some(self.started.elapsed().saturating_sub(Duration::from_millis(at)))
    }
}

/// One open stream. A response body holds it, so the stream counts until the body drops.
pub(crate) struct StreamPermit(Arc<Streams>);

impl StreamPermit {
    pub(crate) fn frame_arrived(&self) {
        self.0.frame_arrived();
    }
}

impl Drop for StreamPermit {
    fn drop(&mut self) {
        self.0.open.fetch_sub(1, Ordering::AcqRel);
        self.0.freed.notify_waiters();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn a_channel_at_its_budget_gives_no_stream_until_one_ends() {
        let streams = Arc::new(Streams::new(2, Arc::new(Notify::new())));
        let (first, _second) = (streams.try_open().expect("a stream"), streams.try_open().expect("a stream"));
        assert!(streams.try_open().is_none());
        assert_eq!((streams.open(), streams.budget(), streams.server_limit()), (2, 2, None));
        drop(first);
        assert!(streams.try_open().is_some());

        streams.set_limit(10, 7);
        assert_eq!((streams.budget(), streams.server_limit()), (7, Some(10)));
        assert!(streams.try_open().is_some(), "a larger budget gives more streams");
    }

    #[tokio::test(start_paused = true)]
    async fn a_channel_retires_only_with_no_stream_and_then_takes_none() {
        let streams = Arc::new(Streams::new(2, Arc::new(Notify::new())));
        let stream = streams.try_open().expect("a stream");
        assert!(!streams.retire(), "a stream is open");
        drop(stream);
        assert!(streams.retire());
        assert!(streams.is_retired() && streams.try_open().is_none());
        assert_eq!(streams.open(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn the_age_of_the_last_frame_counts_from_the_frame() {
        let streams = Arc::new(Streams::new(2, Arc::new(Notify::new())));
        assert_eq!(streams.last_frame_age(), None);
        tokio::time::sleep(Duration::from_secs(3)).await;
        streams.try_open().expect("a stream").frame_arrived();
        tokio::time::sleep(Duration::from_secs(4)).await;
        assert_eq!(streams.last_frame_age(), Some(Duration::from_secs(4)));
    }
}
