//! The report that the start function gives for each run that ended.

use std::fmt;
use std::time::Duration;

use super::status::{End, TaskId, TaskName};

/// One run of a task ended. The journal makes one entry from each report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskReport {
    /// The identity of the task.
    pub id: TaskId,
    /// The name of the task.
    pub task: TaskName,
    /// Why the run ended.
    pub end: End,
    /// The pause before the next run. `None` when the task does not run again.
    pub restart_in: Option<Duration>,
}

impl fmt::Display for TaskReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "task {} {}", self.task, self.end)?;
        match self.restart_in {
            Some(pause) => write!(f, "; restart in {} ms", pause.as_millis()),
            None => f.write_str("; no restart"),
        }
    }
}

/// Takes the reports of all tasks. The journal implements it.
///
/// `report` runs in the supervisor of the task, so it must return fast. A panic in it cannot
/// stop the task: the start function catches the panic and counts the lost report.
pub trait Reporter: Send + Sync + 'static {
    /// Records one report.
    fn report(&self, report: TaskReport);
}

impl<F> Reporter for F
where
    F: Fn(TaskReport) + Send + Sync + 'static,
{
    fn report(&self, report: TaskReport) {
        self(report);
    }
}
