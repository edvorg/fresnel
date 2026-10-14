//! The jobs of the operations (plan section 7.3): the list that the Operations tab shows.
//!
//! Each job is a task of its own that runs one time, so a long job such as a rollout
//! status does not hold the others, and the jobs of a bulk action run side by side. A job
//! that ends gets a journal entry. A job whose task stops before its end, by a panic or by
//! the stop of the core, gets the state "result not known": it can have sent a write.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use fresnel_net::task::{Restart, TaskHandle, TaskName, Tasks};
use tokio::sync::watch;

use super::access::AccessHandle;
use super::repaint::Repaint;
use super::{Inner, Phase};
use crate::error::CoreError;
use crate::journal::Journal;
use crate::ops::{self, Cancel, JobState, Order, Parts, Progress, Report};

/// The most jobs that the list keeps. A job that ended leaves first; the count of those
/// that left shows in the view (design rule 12).
const JOB_LIMIT: usize = 500;

/// One job as the view sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct JobView {
    /// A number that grows with each job of the core. A cancel names it.
    pub number: u64,
    /// The operation and its object.
    pub order: Order,
    /// The state, the steps and the outcome.
    pub report: Report,
}

/// The jobs, the oldest first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct JobsView {
    /// Each job of the list.
    pub jobs: Vec<Arc<JobView>>,
    /// The count of jobs that ended and left the list because of its limit.
    pub dropped: u64,
}

type Slot = Arc<Mutex<Arc<JobView>>>;

fn lock(slot: &Slot) -> MutexGuard<'_, Arc<JobView>> {
    slot.lock().unwrap_or_else(PoisonError::into_inner)
}

struct Entry {
    slot: Slot,
    cancel: watch::Sender<bool>,
    _task: Option<TaskHandle>,
}

/// The jobs of the core.
#[derive(Default)]
pub(super) struct Jobs {
    entries: VecDeque<Entry>,
    number: u64,
    dropped: u64,
}

/// What a job of a connected cluster gets from it: the parts of its procedure, and the
/// permissions of the user, which a refusal with HTTP 403 makes old.
pub(super) type ClusterParts = (Parts, AccessHandle);

impl Jobs {
    /// Starts a job. With no `cluster`, the cluster of the order is not connected, and the
    /// job ends at once as not sent. `cancel` sets the signal of the parts. A list of running
    /// jobs only takes no new one: the job ends as not sent, and only the journal tells it.
    pub(super) fn start(
        &mut self,
        tasks: &Tasks,
        order: Order,
        cluster: Option<ClusterParts>,
        cancel: watch::Sender<bool>,
        ends: Ends,
    ) {
        self.number += 1;
        let report = Report::default();
        let slot: Slot = Arc::new(Mutex::new(Arc::new(JobView { number: self.number, order: order.clone(), report })));
        if let Err(error) = self.make_room() {
            end_now(&slot, JobState::NotSent(error), &ends);
            return;
        }
        let task = match cluster {
            Some((parts, access)) => Some(start(tasks, order, parts, access, &slot, ends)),
            None => {
                let error = CoreError::invalid(format!("{} is not connected", order.target.cluster));
                end_now(&slot, JobState::NotSent(error), &ends);
                None
            }
        };
        self.entries.push_back(Entry { slot, cancel, _task: task });
    }

    /// Asks a job to stop. A wait ends at once; a write in progress runs to its answer.
    pub(super) fn cancel(&self, number: u64) {
        let entry = self.entries.iter().find(|entry| lock(&entry.slot).number == number);
        if let Some(entry) = entry {
            entry.cancel.send_replace(true);
        }
    }

    /// A copy of the list for the view.
    pub(super) fn view(&self) -> Arc<JobsView> {
        let jobs = self.entries.iter().map(|entry| Arc::clone(&lock(&entry.slot))).collect();
        Arc::new(JobsView { jobs, dropped: self.dropped })
    }

    /// Removes the oldest job that ended when the list is full. A list of running jobs
    /// only takes no new one: their tasks would stop.
    fn make_room(&mut self) -> Result<(), CoreError> {
        if self.entries.len() < JOB_LIMIT {
            return Ok(());
        }
        let ended = self.entries.iter().position(|entry| lock(&entry.slot).report.state.is_final());
        let ended = ended.ok_or_else(|| CoreError::invalid(format!("{JOB_LIMIT} jobs run: the job waits for none")))?;
        self.entries.remove(ended);
        self.dropped += 1;
        Ok(())
    }
}

/// What takes the end of each job: the journal and the repaint.
#[derive(Clone)]
pub(super) struct Ends {
    pub(super) journal: Journal,
    pub(super) repaint: Repaint,
}

impl Ends {
    fn record(&self, view: &JobView) {
        let target = &view.order.target;
        let (kind, key) = (target.kind.info().name, &target.key);
        let object =
            if key.namespace.is_empty() { key.name.to_string() } else { format!("{}/{}", key.namespace, key.name) };
        let what = format!("{} of {kind} {object}", view.order.operation.label());
        match &view.report.state {
            JobState::Done => self.journal.info(target.cluster.as_str(), format!("{what}: done")),
            JobState::Cancelled => self.journal.info(target.cluster.as_str(), format!("{what}: cancelled")),
            JobState::Pending => {}
            JobState::Failed(error) => {
                self.journal.warning(target.cluster.as_str(), format!("{what}: failed: {error}"))
            }
            JobState::NotSent(error) => {
                self.journal.warning(target.cluster.as_str(), format!("{what}: not sent: {error}"))
            }
            JobState::Unknown(error) => {
                self.journal.error(target.cluster.as_str(), format!("{what}: the result is not known: {error}"));
            }
        }
        self.repaint.request();
    }
}

/// The view of the job takes each report of the procedure.
struct Shown {
    slot: Slot,
    ends: Ends,
}

impl Progress for Shown {
    fn show(&mut self, report: &Report) {
        {
            let mut view = lock(&self.slot);
            *view = Arc::new(JobView { report: report.clone(), ..JobView::clone(&view) });
        }
        self.ends.repaint.request();
    }
}

impl Drop for Shown {
    /// A task that stops before the end of its procedure leaves a job that is not final.
    /// When it can have sent a write, its result is not known (design rule 6).
    fn drop(&mut self) {
        let (pending, writes) = {
            let view = lock(&self.slot);
            (!view.report.state.is_final(), view.order.operation.writes())
        };
        if pending {
            let error = CoreError::invalid("the job stopped before its end");
            let state = if writes { JobState::Unknown(error) } else { JobState::Failed(error) };
            end_now(&self.slot, state, &self.ends);
        }
    }
}

fn start(tasks: &Tasks, order: Order, parts: Parts, access: AccessHandle, slot: &Slot, ends: Ends) -> TaskHandle {
    let name = TaskName::scoped("job", order.target.cluster.as_str());
    let slot = Arc::clone(slot);
    tasks.start(name, Restart::Never, move || {
        let (order, parts, access) = (order.clone(), parts.clone(), access.clone());
        let mut shown = Shown { slot: Arc::clone(&slot), ends: ends.clone() };
        async move {
            let state = ops::run(&order, parts, &mut shown).await;
            // The rules said yes, and the server said no: the rules of the namespace are old.
            if state.error().is_some_and(|error| error.status() == Some(403)) {
                access.forget(&order.target.key.namespace);
            }
            shown.ends.record(&lock(&shown.slot));
            Ok::<(), CoreError>(())
        }
    })
}

fn end_now(slot: &Slot, state: JobState, ends: &Ends) {
    let view = {
        let mut view = lock(slot);
        let mut report = view.report.clone();
        report.state = state;
        *view = Arc::new(JobView { report, ..JobView::clone(&view) });
        Arc::clone(&view)
    };
    ends.record(&view);
}

impl Inner {
    /// Starts a job for each order: one for an action, one for each row of a bulk action.
    pub(super) fn run_orders(&self, orders: Vec<Order>) {
        let ends = Ends { journal: self.journal.clone(), repaint: self.repaint.clone() };
        let mut state = self.state();
        if state.phase == Phase::Stopped {
            return;
        }
        for order in orders {
            let (cancel, signal) = Cancel::pair();
            let cluster = state.connected.get(&order.target.cluster).map(|connected| connected.job_parts(signal));
            state.jobs.start(&self.tasks, order, cluster, cancel, ends.clone());
        }
        drop(state);
        self.repaint.request();
    }
}

#[cfg(test)]
mod tests;
