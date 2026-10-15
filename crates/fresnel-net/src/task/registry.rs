//! The states of all started tasks.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::SystemTime;

use super::status::{End, TaskId, TaskName, TaskState, TaskStatus};

/// Holds one status for each task whose handle is alive.
///
/// Each method holds the lock only for one small change or one copy. Thus a reader of the
/// states can never stop a task.
pub(super) struct Registry {
    next_id: AtomicU64,
    tasks: Mutex<BTreeMap<TaskId, TaskStatus>>,
}

impl Registry {
    pub(super) fn new() -> Self {
        Self { next_id: AtomicU64::new(0), tasks: Mutex::new(BTreeMap::new()) }
    }

    pub(super) fn add(&self, name: TaskName) -> TaskId {
        let id = TaskId(self.next_id.fetch_add(1, Ordering::Relaxed));
        let status = TaskStatus { id, name, state: TaskState::Running, runs: 0, since: SystemTime::now() };
        self.lock().insert(id, status);
        id
    }

    pub(super) fn remove(&self, id: TaskId) {
        self.lock().remove(&id);
    }

    pub(super) fn run_started(&self, id: TaskId) {
        self.change(id, |status| {
            status.runs = status.runs.saturating_add(1);
            status.enter(TaskState::Running);
        });
    }

    /// Records the end of a run. False for a task whose handle dropped: its owner stopped it.
    pub(super) fn run_ended(&self, id: TaskId, state: TaskState) -> bool {
        let mut tasks = self.lock();
        let Some(status) = tasks.get_mut(&id) else {
            return false;
        };
        status.enter(state);
        true
    }

    /// Records that the supervisor of the task is gone. An end that is already recorded stays.
    pub(super) fn supervisor_gone(&self, id: TaskId) {
        self.change(id, |status| {
            if !matches!(status.state, TaskState::Ended(_)) {
                status.enter(TaskState::Ended(End::Stopped));
            }
        });
    }

    pub(super) fn snapshot(&self) -> Vec<TaskStatus> {
        self.lock().values().cloned().collect()
    }

    /// Changes the status of a task. A task whose handle dropped has no status, and nothing changes.
    fn change(&self, id: TaskId, change: impl FnOnce(&mut TaskStatus)) {
        if let Some(status) = self.lock().get_mut(&id) {
            change(status);
        }
    }

    /// A poisoned lock is safe to use here: each change under the lock is complete or not started.
    fn lock(&self) -> MutexGuard<'_, BTreeMap<TaskId, TaskStatus>> {
        self.tasks.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
