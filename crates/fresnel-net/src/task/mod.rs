//! The one place that starts background tasks (design rule 9).
//!
//! In each prototype, a task that ended with no visible reason left old data on the screen.
//! Thus the start function gives each task a name, a state that the UI can read, a report for
//! the journal when a run ends, and a restart with backoff. The lint `disallowed_methods`
//! refuses a direct `tokio::spawn` and a direct `std::thread::spawn`.
//!
//! [`Tasks::start`] runs a task on the runtime. [`Tasks::start_thread`] gives a task a
//! thread of its own, for work that computes for a long time.

mod backoff;
mod registry;
mod report;
mod status;
mod supervise;
mod thread;

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
pub use thread::Stop;

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
        TaskHandle { id, runner: Runner::Runtime(running.abort_handle()), shared: Arc::clone(&self.shared) }
    }

    /// Starts a task on a thread of its own, for work that computes for a long time, such
    /// as the rows of a table. `body` is one run, and each restart calls it again.
    ///
    /// Nothing can stop a thread from outside. A body that waits for its input must end when
    /// that input ends, and its owner must drop the handle before the input. A body that
    /// computes for a long time reads the [`Stop`] between two steps.
    #[must_use = "the task stops when its handle drops"]
    pub fn start_thread<B, E>(&self, name: TaskName, restart: Restart, body: B) -> TaskHandle
    where
        B: FnMut(&Stop) -> Result<(), E> + Send + 'static,
        E: Display,
    {
        let id = self.shared.registry.add(name.clone());
        let stop = Arc::new(Stop::default());
        let supervisor = Supervisor { shared: Arc::clone(&self.shared), id, name: name.clone(), restart };
        // The standard library panics for a thread name with a NUL byte, and the scope of a
        // task can be a name from a file of the user.
        let thread = std::thread::Builder::new().name(name.to_string().replace('\0', " "));
        #[expect(clippy::disallowed_methods, reason = "this is the one start function for a thread")]
        let started = thread.spawn({
            let stop = Arc::clone(&stop);
            move || supervisor.run_on_thread(body, &stop)
        });
        // The thread is not joined: it ends by itself after the stop.
        if let Err(error) = started {
            let end = End::Failed(format!("the system gave no thread: {error}"));
            self.shared.registry.run_ended(id, TaskState::Ended(end.clone()));
            self.shared.report(TaskReport { id, task: name, end, restart_in: None });
        }
        TaskHandle { id, runner: Runner::Thread(stop), shared: Arc::clone(&self.shared) }
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
    runner: Runner,
    shared: Arc<Shared>,
}

/// What runs the supervisor of a task, with the way to stop it.
enum Runner {
    /// A task of the runtime. The abort ends it at its next wait.
    Runtime(AbortHandle),
    /// A thread. It ends when its body returns after the stop.
    Thread(Arc<Stop>),
}

impl TaskHandle {
    /// The identity of the task in [`Tasks::snapshot`].
    pub fn id(&self) -> TaskId {
        self.id
    }
}

impl Drop for TaskHandle {
    /// The state goes first. If the stop went first, a reader on a different thread could see
    /// the stopped supervisor as a task with the final state "stopped".
    fn drop(&mut self) {
        self.shared.registry.remove(self.id);
        match &self.runner {
            Runner::Runtime(supervisor) => supervisor.abort(),
            Runner::Thread(stop) => stop.set(),
        }
    }
}

impl fmt::Debug for TaskHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TaskHandle").field("id", &self.id).finish_non_exhaustive()
    }
}
