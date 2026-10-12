//! The one signal that ends each retry delay of a cluster.
//!
//! After a wake, a network change or "retry now", a wait for a backoff is lost time. Each
//! part of a cluster that waits before a retry listens to this signal.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;

/// Why the signal was raised. Each listener does the same for each cause; the cause is for
/// the state that the UI shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cause {
    /// The computer slept.
    Wake,
    /// An interface address or the default route changed.
    NetworkChange,
    /// The user asked for a retry.
    RetryNow,
}

/// Raises the signal of one cluster. A clone raises the same signal.
#[derive(Clone, Debug)]
pub struct Signal {
    raised: Arc<watch::Sender<(u64, Cause)>>,
}

impl Default for Signal {
    fn default() -> Self {
        Self::new()
    }
}

impl Signal {
    /// A signal that nobody raised.
    pub fn new() -> Self {
        Self { raised: Arc::new(watch::Sender::new((0, Cause::RetryNow))) }
    }

    /// Ends each retry delay of the cluster now.
    pub fn raise(&self, cause: Cause) {
        self.raised.send_modify(|(count, last)| {
            *count = count.wrapping_add(1);
            *last = cause;
        });
    }

    /// A listener that hears each raise after this call.
    pub fn listen(&self) -> Listener {
        Listener { raised: self.raised.subscribe() }
    }
}

/// Hears the raises of one [`Signal`]. Two raises before a read count as one.
#[derive(Debug)]
pub struct Listener {
    raised: watch::Receiver<(u64, Cause)>,
}

impl Listener {
    /// Waits for the next raise. It waits with no end after each `Signal` dropped: the
    /// cluster is then gone, and its tasks stop.
    pub async fn raised(&mut self) -> Cause {
        if self.raised.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
        self.raised.borrow_and_update().1
    }

    /// Waits for `pause`, or less when the signal is raised. Gives the cause of an early end.
    pub async fn sleep(&mut self, pause: Duration) -> Option<Cause> {
        tokio::select! {
            cause = self.raised() => Some(cause),
            () = tokio::time::sleep(pause) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use tokio::time::Instant;

    use super::*;

    #[tokio::test(start_paused = true)]
    async fn a_raise_ends_the_pause_of_each_listener() {
        let signal = Signal::new();
        let (mut first, mut second) = (signal.listen(), signal.listen());
        let started = Instant::now();
        let raise = async {
            tokio::time::sleep(Duration::from_secs(1)).await;
            signal.clone().raise(Cause::Wake);
        };
        let pause = Duration::from_secs(60);
        let (one, two, ()) = tokio::join!(first.sleep(pause), second.sleep(pause), raise);
        assert_eq!((one, two), (Some(Cause::Wake), Some(Cause::Wake)));
        assert_eq!(started.elapsed(), Duration::from_secs(1));
    }

    #[tokio::test(start_paused = true)]
    async fn a_pause_with_no_raise_has_its_full_length_and_an_old_raise_does_not_count() {
        let signal = Signal::new();
        signal.raise(Cause::NetworkChange);
        let mut listener = signal.listen();
        let started = Instant::now();
        assert_eq!(listener.sleep(Duration::from_secs(3)).await, None);
        assert_eq!(started.elapsed(), Duration::from_secs(3));

        signal.raise(Cause::Wake);
        signal.raise(Cause::RetryNow);
        assert_eq!(listener.sleep(Duration::from_secs(3)).await, Some(Cause::RetryNow), "two raises count as one");
        assert_eq!(listener.sleep(Duration::from_secs(3)).await, None);
    }
}
