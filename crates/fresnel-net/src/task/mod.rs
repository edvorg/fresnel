//! The one place that starts background tasks (design rule 9).
//!
//! In each prototype, a task that ended with no visible reason left old data on the screen.
//! Thus the start function gives each task a name, a state that the UI can read, a report for
//! the journal when a run ends, and a restart with backoff. The lint `disallowed_methods`
//! refuses a direct `tokio::spawn`.

mod backoff;
mod registry;
mod report;
mod status;
mod supervise;

use std::fmt::{self, Display};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::runtime::Handle;
use tokio::task::AbortHandle;

pub use backoff::Backoff;
use registry::Registry;
pub use report::{Reporter, TaskReport};
pub use status::{End, TaskId, TaskName, TaskState, TaskStatus};
use supervise::Supervisor;

/// What follows the end of one run of a task.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Restart {
    /// The task runs one time. The end of the run is its final state.
    Never,
    /// The task runs again after each end, also after `Ok`: a long-lived task must not end.
    Always(Backoff),
}

/// Starts all background tasks and holds their states.
#[derive(Clone)]
pub struct Tasks {
    runtime: Handle,
    shared: Arc<Shared>,
}

struct Shared {
    registry: Registry,
    reporter: Box<dyn Reporter>,
    lost_reports: AtomicU64,
}

impl Tasks {
    /// `runtime` runs the tasks. Its timer must be on, because a restart waits on it. With no
    /// timer, the first restart ends the task with a panic as its final state.
    /// `reporter` takes one report for each run that ends.
    pub fn new(runtime: Handle, reporter: impl Reporter) -> Self {
        let shared =
            Shared { registry: Registry::new(), reporter: Box::new(reporter), lost_reports: AtomicU64::new(0) };
        Self { runtime, shared: Arc::new(shared) }
    }

    /// Starts a task. `body` builds one run, and each restart calls it again.
    ///
    /// The caller needs no runtime context, so the UI thread can start a task.
    #[must_use = "the task stops when its handle drops"]
    pub fn start<B, F, E>(&self, name: TaskName, restart: Restart, body: B) -> TaskHandle
    where
        B: FnMut() -> F + Send + 'static,
        F: Future<Output = Result<(), E>> + Send + 'static,
        E: Display + 'static,
    {
        let id = self.shared.registry.add(name.clone());
        let supervisor = Supervisor { shared: Arc::clone(&self.shared), id, name, restart };
        #[expect(clippy::disallowed_methods, reason = "this is the one start function")]
        let running = self.runtime.spawn(supervisor.run(body));
        TaskHandle { id, supervisor: running.abort_handle(), shared: Arc::clone(&self.shared) }
    }

    /// A copy of the state of each task, in the sequence of their start.
    pub fn snapshot(&self) -> Vec<TaskStatus> {
        self.shared.registry.snapshot()
    }

    /// The count of reports that the reporter lost because it panicked.
    pub fn lost_reports(&self) -> u64 {
        self.shared.lost_reports.load(Ordering::Relaxed)
    }
}

impl Shared {
    /// Gives a report to the reporter. A reporter that panics must not stop the supervisor.
    fn report(&self, report: TaskReport) {
        if catch_unwind(AssertUnwindSafe(|| self.reporter.report(report))).is_err() {
            self.lost_reports.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Owns one started task. The task stops when the handle drops, so no task lives longer
/// than its owner. Its state then leaves the list of [`Tasks::snapshot`].
pub struct TaskHandle {
    id: TaskId,
    supervisor: AbortHandle,
    shared: Arc<Shared>,
}

impl TaskHandle {
    /// The identity of the task in [`Tasks::snapshot`].
    pub fn id(&self) -> TaskId {
        self.id
    }
}

impl Drop for TaskHandle {
    fn drop(&mut self) {
        self.supervisor.abort();
        self.shared.registry.remove(self.id);
    }
}

impl fmt::Debug for TaskHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TaskHandle").field("id", &self.id).finish_non_exhaustive()
    }
}
