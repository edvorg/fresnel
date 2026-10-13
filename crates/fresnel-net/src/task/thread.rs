//! A task on a thread of its own, for work that computes for a long time.
//!
//! The rows of a large table need some milliseconds, and such work must not hold a thread
//! of the runtime or a frame of the UI. In the UI prototype, a worker thread that ended
//! with a panic left a table that showed "updating" with no end. Thus a thread gets the
//! same state, report and restart as each other task.
//!
//! Nothing can stop a thread from outside. The owner tells the thread to stop, and the
//! body ends by itself: its input ends, or it reads the [`Stop`] between two steps.

use std::fmt::Display;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use super::status::End;
use super::supervise::{Supervisor, panic_message};

/// Tells the body of a thread task that its owner dropped the handle of the task.
#[derive(Debug, Default)]
pub struct Stop {
    set: Mutex<bool>,
    changed: Condvar,
}

impl Stop {
    /// True after the owner dropped the handle. A body that computes for a long time reads
    /// this between two steps and returns.
    pub fn is_set(&self) -> bool {
        *self.set.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(super) fn set(&self) {
        *self.set.lock().unwrap_or_else(PoisonError::into_inner) = true;
        self.changed.notify_all();
    }

    /// Waits for `pause`, or less when the owner stops the task. True after a stop.
    fn wait(&self, pause: Duration) -> bool {
        let set = self.set.lock().unwrap_or_else(PoisonError::into_inner);
        let waited = self.changed.wait_timeout_while(set, pause, |set| !*set);
        *waited.unwrap_or_else(PoisonError::into_inner).0
    }
}

impl Supervisor {
    /// Runs the task on the current thread until its final end or its stop.
    ///
    /// A panic of the supervisor itself also gets a final state and a report here, because
    /// nothing reads the result of the thread.
    pub(super) fn run_on_thread<B, E>(self, mut body: B, stop: &Stop)
    where
        B: FnMut(&Stop) -> Result<(), E>,
        E: Display,
    {
        if let Err(panic) = catch_unwind(AssertUnwindSafe(|| self.run_and_restart_on_thread(&mut body, stop))) {
            self.end(End::Panicked(panic_message(panic.as_ref())));
        }
    }

    fn run_and_restart_on_thread<B, E>(&self, body: &mut B, stop: &Stop)
    where
        B: FnMut(&Stop) -> Result<(), E>,
        E: Display,
    {
        let mut early_ends = 0_u32;
        loop {
            self.shared.registry.run_started(self.id);
            let started = Instant::now();
            let end = run_once(body, stop);
            // The owner stopped the task. Its state is gone, and this end is not a fault.
            if stop.is_set() {
                return;
            }
            let Some(pause) = self.after_run(end, started.elapsed(), &mut early_ends) else {
                return;
            };
            if stop.wait(pause) {
                return;
            }
        }
    }
}

/// Runs the body one time. All code of the caller runs inside the panic guard.
fn run_once<B, E>(body: &mut B, stop: &Stop) -> End
where
    B: FnMut(&Stop) -> Result<(), E>,
    E: Display,
{
    match catch_unwind(AssertUnwindSafe(|| body(stop).map_err(|error| error.to_string()))) {
        Ok(Ok(())) => End::Done,
        Ok(Err(error)) => End::Failed(error),
        Err(panic) => End::Panicked(panic_message(panic.as_ref())),
    }
}
