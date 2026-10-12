//! The detectors of a wake and of a network change (plan section 8.12).
//!
//! After the computer slept or the network changed, each open connection is in doubt, and
//! each wait for a retry is lost time. The detectors are behind one trait, so that the task
//! that reads them is one loop, and a test can give its own detector.

mod clock_jump;
mod network;

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::time::{MissedTickBehavior, interval};

pub use clock_jump::ClockJump;
pub use network::NetworkChange;

use crate::error::NetError;
use crate::signal::Cause;
use crate::task::{Backoff, Restart, TaskHandle, TaskName, Tasks};

/// Something made each connection of the app doubtful.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disturbance {
    /// The wall clock moved this much more than the monotonic clock: the computer slept.
    Wake(Duration),
    /// An interface address or the default route changed.
    NetworkChange,
}

impl Disturbance {
    /// The cause for the signal of a cluster.
    pub fn cause(self) -> Cause {
        match self {
            Self::Wake(_) => Cause::Wake,
            Self::NetworkChange => Cause::NetworkChange,
        }
    }
}

/// Looks at the system for one type of disturbance.
pub trait Detector: Send + 'static {
    /// Looks one time. Gives a disturbance if one occurred after the look before. The call
    /// must return fast: it runs on a thread of the runtime.
    fn look(&mut self) -> Option<Disturbance>;
}

/// The two detectors of the app on the real system.
pub fn system_detectors() -> Vec<Box<dyn Detector>> {
    vec![Box::new(ClockJump::system()), Box::new(NetworkChange::system())]
}

/// The task that reads the detectors. It stops when this value drops.
#[derive(Debug)]
pub struct Monitor {
    _task: TaskHandle,
}

impl Monitor {
    /// Reads each detector each time `period` ends, and gives each disturbance to `report`.
    /// The app raises the signal of each cluster there.
    pub fn start(
        tasks: &Tasks,
        detectors: Vec<Box<dyn Detector>>,
        period: Duration,
        report: impl Fn(Disturbance) + Send + Sync + 'static,
    ) -> Self {
        let detectors = Arc::new(Mutex::new(detectors));
        let report: Arc<dyn Fn(Disturbance) + Send + Sync> = Arc::new(report);
        let body = move || look_each_period(Arc::clone(&detectors), period, Arc::clone(&report));
        Self {
            _task: tasks.start(TaskName::new("wake and network monitor"), Restart::Always(Backoff::default()), body),
        }
    }
}

async fn look_each_period(
    detectors: Arc<Mutex<Vec<Box<dyn Detector>>>>,
    period: Duration,
    report: Arc<dyn Fn(Disturbance) + Send + Sync>,
) -> Result<(), NetError> {
    let mut ticks = interval(period);
    // After a sleep of the computer, one look is sufficient.
    ticks.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        ticks.tick().await;
        let seen: Vec<Disturbance> = {
            let mut detectors = detectors.lock().unwrap_or_else(PoisonError::into_inner);
            detectors.iter_mut().filter_map(|detector| detector.look()).collect()
        };
        seen.into_iter().for_each(|disturbance| report(disturbance));
    }
}

#[cfg(test)]
mod tests {
    use tokio::runtime::Handle;
    use tokio::time::{Instant, sleep};

    use super::*;
    use crate::signal::Signal;

    const SECOND: Duration = Duration::from_secs(1);

    /// Sees a disturbance at one of its looks.
    struct Scripted {
        looks: u32,
        sees_at: u32,
        disturbance: Disturbance,
    }

    impl Detector for Scripted {
        fn look(&mut self) -> Option<Disturbance> {
            self.looks += 1;
            (self.looks == self.sees_at).then_some(self.disturbance)
        }
    }

    #[tokio::test(start_paused = true)]
    async fn the_monitor_reads_each_detector_each_period_and_the_report_raises_the_signal() {
        let tasks = Tasks::new(Handle::current(), |report| panic!("the monitor ended: {report}"));
        let signal = Signal::new();
        let mut listener = signal.listen();
        let detectors: Vec<Box<dyn Detector>> = vec![
            Box::new(Scripted { looks: 0, sees_at: 3, disturbance: Disturbance::NetworkChange }),
            Box::new(Scripted { looks: 0, sees_at: 6, disturbance: Disturbance::Wake(600 * SECOND) }),
        ];
        let started = Instant::now();
        let monitor = Monitor::start(&tasks, detectors, SECOND, move |disturbance| signal.raise(disturbance.cause()));

        // The first look is at the start, so look 3 is after 2 s.
        assert_eq!((listener.raised().await, started.elapsed()), (Cause::NetworkChange, 2 * SECOND));
        assert_eq!((listener.raised().await, started.elapsed()), (Cause::Wake, 5 * SECOND));
        drop(monitor);
        sleep(60 * SECOND).await;
        assert_eq!(tasks.snapshot(), Vec::new(), "the task of the monitor stops with its handle");
    }
}
