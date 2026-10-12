//! The supervisor of one task: run, record the end, report it, wait, run again.

use std::any::Any;
use std::fmt::Display;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::Duration;

use futures_util::FutureExt as _;
use tokio::time::{Instant, sleep};

use super::report::TaskReport;
use super::status::{End, TaskId, TaskName, TaskState};
use super::{Restart, Shared};

/// The supervisor of one task. Its drop gives the task a final state for each end that the
/// loop does not handle: a runtime that stops, also before the first run.
pub(super) struct Supervisor {
    pub(super) shared: Arc<Shared>,
    pub(super) id: TaskId,
    pub(super) name: TaskName,
    pub(super) restart: Restart,
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        self.shared.registry.supervisor_gone(self.id);
    }
}

impl Supervisor {
    /// Runs the task until its final end. The state changes before the report, so a reader of
    /// the report always finds the new state.
    pub(super) async fn run<B, F, E>(self, mut body: B)
    where
        B: FnMut() -> F,
        F: Future<Output = Result<(), E>>,
        E: Display,
    {
        let registry = &self.shared.registry;
        let mut early_ends = 0_u32;
        loop {
            registry.run_started(self.id);
            let started = Instant::now();
            let end = run_once(&mut body).await;
            let Restart::Always(backoff) = self.restart else {
                registry.run_ended(self.id, TaskState::Ended(end.clone()));
                self.report(end, None);
                return;
            };
            early_ends = if started.elapsed() >= backoff.reset_after { 1 } else { early_ends.saturating_add(1) };
            let pause = backoff.pause(early_ends, fastrand::u32(..));
            registry.run_ended(self.id, TaskState::Backoff { last: end.clone(), early_ends, pause });
            self.report(end, Some(pause));
            sleep(pause).await;
        }
    }

    fn report(&self, end: End, restart_in: Option<Duration>) {
        self.shared.report(TaskReport { id: self.id, task: self.name.clone(), end, restart_in });
    }
}

/// Runs the body one time. All code of the caller runs inside the panic guard: the call of
/// `body`, the future, and the text of the error.
async fn run_once<B, F, E>(body: &mut B) -> End
where
    B: FnMut() -> F,
    F: Future<Output = Result<(), E>>,
    E: Display,
{
    let run = async { body().await.map_err(|error| error.to_string()) };
    match AssertUnwindSafe(run).catch_unwind().await {
        Ok(Ok(())) => End::Done,
        Ok(Err(error)) => End::Failed(error),
        Err(panic) => End::Panicked(panic_message(panic.as_ref())),
    }
}

fn panic_message(panic: &(dyn Any + Send)) -> String {
    if let Some(message) = panic.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = panic.downcast_ref::<String>() {
        message.clone()
    } else {
        "a panic without a text".to_owned()
    }
}
