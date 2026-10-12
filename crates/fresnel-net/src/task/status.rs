//! The name and the visible state of a task.

use std::fmt;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

/// Identifies one started task. Two tasks can have the same name, but not the same id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TaskId(pub(super) u64);

/// The name of a task: its role and, if it has one, the object that it works for.
///
/// The scope is a text, for example a cluster name. It lets the UI show which cluster a
/// failed task belongs to.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TaskName {
    role: &'static str,
    scope: Option<Arc<str>>,
}

impl TaskName {
    /// The name of a task that exists one time in the app.
    pub const fn new(role: &'static str) -> Self {
        Self { role, scope: None }
    }

    /// The name of a task that exists one time for each scope, for example for each cluster.
    pub fn scoped(role: &'static str, scope: impl Into<Arc<str>>) -> Self {
        Self { role, scope: Some(scope.into()) }
    }
}

impl fmt::Display for TaskName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.scope {
            Some(scope) => write!(f, "{} [{scope}]", self.role),
            None => f.write_str(self.role),
        }
    }
}

/// Why one run of a task ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum End {
    /// The body returned `Ok`.
    Done,
    /// The body returned this error.
    Failed(String),
    /// The body panicked with this message.
    Panicked(String),
    /// The runtime stopped while the task was alive.
    Stopped,
}

impl fmt::Display for End {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Done => f.write_str("ended"),
            Self::Failed(error) => write!(f, "failed: {error}"),
            Self::Panicked(message) => write!(f, "panicked: {message}"),
            Self::Stopped => f.write_str("stopped"),
        }
    }
}

/// The state of a task that the UI shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TaskState {
    /// A run is in progress.
    Running,
    /// The last run ended with `last`. The next run starts after `pause`.
    Backoff {
        /// The end of the last run.
        last: End,
        /// The count of runs in sequence that ended early.
        early_ends: u32,
        /// The pause from the state change to the next run.
        pause: Duration,
    },
    /// The task does not run again.
    Ended(End),
}

/// A copy of the state of one task.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskStatus {
    /// The identity of the task.
    pub id: TaskId,
    /// The name of the task.
    pub name: TaskName,
    /// The current state.
    pub state: TaskState,
    /// The count of runs that started.
    pub runs: u32,
    /// The wall clock time of the last state change.
    pub since: SystemTime,
}

impl TaskStatus {
    /// Changes the state and records the time of the change.
    pub(super) fn enter(&mut self, state: TaskState) {
        self.state = state;
        self.since = SystemTime::now();
    }
}
