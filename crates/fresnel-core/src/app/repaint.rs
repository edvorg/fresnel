//! The one notifier that asks the UI for a repaint (plan section 7.3).
//!
//! A state can change many times in one second. The notifier asks for a limited count of
//! repaints, and the last change always gets one. The core does not know egui: the window
//! gives the function that wakes it.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use fresnel_net::task::{Reporter, TaskReport};
use tokio::sync::Notify;

use crate::error::CoreError;
use crate::journal::Journal;

/// The journal takes the report of each task run that ended (design rule 1), and the UI
/// gets a repaint for the new entry.
pub(super) fn reporter(journal: Journal, repaint: Repaint) -> impl Reporter {
    move |report: TaskReport| {
        journal.report(report);
        repaint.request();
    }
}

/// Wakes the UI. The window gives a function that asks egui for a repaint.
pub(super) type Wake = Arc<dyn Fn() + Send + Sync>;

/// The most wakes in one second until the program or the settings give a limit.
const PER_SECOND: u32 = 60;

struct Shared {
    changed: Notify,
    wake: OnceLock<Wake>,
    /// The most wakes in one second. The settings file can change it while the task runs.
    per_second: AtomicU32,
}

/// Collects the requests for a repaint. A clone is one more handle.
#[derive(Clone)]
pub(super) struct Repaint {
    shared: Arc<Shared>,
}

impl Repaint {
    pub(super) fn new() -> Self {
        let shared = Shared { changed: Notify::new(), wake: OnceLock::new(), per_second: AtomicU32::new(PER_SECOND) };
        Self { shared: Arc::new(shared) }
    }

    /// Sets the most wakes in one second, 1 at least. It acts after the next wake.
    pub(super) fn set_limit(&self, per_second: u32) {
        self.shared.per_second.store(per_second.max(1), Ordering::Relaxed);
    }

    /// The most wakes in one second now, for a test of the settings.
    #[cfg(test)]
    pub(super) fn limit(&self) -> u32 {
        self.shared.per_second.load(Ordering::Relaxed)
    }

    /// Tells that a state changed. The call returns at once and never runs code of the UI.
    pub(super) fn request(&self) {
        self.shared.changed.notify_one();
    }

    /// Sets the function that wakes the UI. Only the first call counts.
    pub(super) fn set_wake(&self, wake: Wake) {
        self.shared.wake.get_or_init(|| wake);
    }

    /// The body of the notifier task: one wake for all requests of one period.
    pub(super) async fn run(self) -> Result<(), CoreError> {
        loop {
            self.shared.changed.notified().await;
            if let Some(wake) = self.shared.wake.get() {
                wake();
            }
            let per_second = self.shared.per_second.load(Ordering::Relaxed).max(1);
            tokio::time::sleep(Duration::from_secs(1) / per_second).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use fresnel_net::task::{Backoff, Restart, TaskName, TaskReport, Tasks};
    use tokio::runtime::Handle;
    use tokio::time::{Instant, sleep};

    use super::*;

    #[tokio::test(start_paused = true)]
    async fn many_requests_in_one_period_give_one_wake_and_the_last_request_gets_one() {
        let tasks = Tasks::new(Handle::current(), |report: TaskReport| panic!("the notifier ended: {report}"));
        let repaint = Repaint::new();
        repaint.set_limit(100);
        let wakes = Arc::new(AtomicU32::new(0));
        let count = Arc::clone(&wakes);
        repaint.set_wake(Arc::new(move || {
            count.fetch_add(1, Ordering::Relaxed);
        }));
        repaint.set_wake(Arc::new(|| panic!("only the first function counts")));
        let run = {
            let repaint = repaint.clone();
            move || repaint.clone().run()
        };
        let _task = tasks.start(TaskName::new("repaint notifier"), Restart::Always(Backoff::default()), run);

        let started = Instant::now();
        // One request each millisecond for 95 ms: 100 requests for each second of the limit.
        for _ in 0..95 {
            repaint.request();
            sleep(Duration::from_millis(1)).await;
        }
        assert_eq!(wakes.load(Ordering::Relaxed), 10, "one wake for each period of 10 ms in {:?}", started.elapsed());

        sleep(Duration::from_secs(60)).await;
        assert_eq!(wakes.load(Ordering::Relaxed), 11, "the request in the last period got its wake");
        sleep(Duration::from_secs(60)).await;
        assert_eq!(wakes.load(Ordering::Relaxed), 11, "no request gives no wake");

        // A new limit of the settings acts after the next wake: 10 in one second.
        repaint.set_limit(10);
        for _ in 0..951 {
            repaint.request();
            sleep(Duration::from_millis(1)).await;
        }
        assert_eq!(wakes.load(Ordering::Relaxed), 21, "one wake for each period of 100 ms in 951 ms");
    }
}
