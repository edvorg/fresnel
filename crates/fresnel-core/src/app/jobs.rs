//! The jobs of the operations (plan section 7.3): the list that the Operations tab shows.
//!
//! Each job is a task of its own that runs one time, so a long job such as a rollout
//! status does not hold the others, and the jobs of a bulk action run side by side. A job
//! that ends gets a journal entry. A job whose task stops before its end, by a panic or by
//! the stop of the core, gets the state "result not known": it can have sent a write.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use fresnel_net::ErrorClass;
use fresnel_net::task::{Restart, TaskHandle, TaskName, Tasks};
use tokio::sync::watch;

use super::access::AccessHandle;
use super::repaint::Repaint;
use super::{Inner, Phase};
use crate::error::CoreError;
use crate::journal::Journal;
use crate::ops::{self, After, Cancel, JobState, Order, Parts, Progress, Report};

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

impl JobsView {
    /// How many new jobs the list takes now. A new job pushes out a job that ended, never
    /// one that runs, so a bulk action on more objects would end the rest as not sent, with
    /// only a journal entry: the confirm dialog refuses it first.
    pub fn room(&self) -> usize {
        let running = self.jobs.iter().filter(|job| !job.report.state.is_final()).count();
        JOB_LIMIT.saturating_sub(running)
    }
}

type Slot = Arc<Mutex<Arc<JobView>>>;
/// Set when the job ends: a job of the next document waits for it.
type Ended = Arc<watch::Sender<bool>>;

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
    /// Starts a job. With an error in place of `cluster`, the cluster of the order is not
    /// connected or makes no change, and the job ends at once as not sent with it. `cancel`
    /// sets the signal of the parts. A list of running jobs only takes no new one: the job
    /// ends as not sent, and only the journal tells it.
    ///
    /// The same order while its job runs ends at once as not sent, as a fast second click
    /// gives it: a second "roll back to the revision before" would undo the first one, and
    /// a second "run again" would race the first one for the name of the Job.
    ///
    /// The job waits for the end of `after` first. Gives the end that a next job waits for:
    /// the end of this job, or `after` when this job ended at once, so a document after a
    /// copy of the one before still waits for that one.
    pub(super) fn start(
        &mut self,
        tasks: &Tasks,
        (order, cluster): (Order, Result<ClusterParts, CoreError>),
        cancel: watch::Sender<bool>,
        ends: Ends,
        mut after: Option<After>,
    ) -> After {
        self.number += 1;
        let report = Report::default();
        let slot: Slot = Arc::new(Mutex::new(Arc::new(JobView { number: self.number, order: order.clone(), report })));
        let (sender, receiver) = watch::channel(false);
        let (ended, own) = (Arc::new(sender), After { number: self.number, ended: receiver });
        if let Err(error) = self.make_room() {
            end_now(&slot, JobState::NotSent(error), &ends, &ended);
            return after.unwrap_or(own);
        }
        let task = match (self.running(&order), cluster) {
            (Some(number), _) => {
                let error = CoreError::invalid(format!("job {number} runs the same operation on the object"));
                end_now(&slot, JobState::NotSent(error), &ends, &ended);
                None
            }
            (None, Ok((mut parts, access))) => {
                parts.after = after.take();
                Some(start(tasks, order, (parts, access), (&slot, ends, ended)))
            }
            (None, Err(error)) => {
                end_now(&slot, JobState::NotSent(error), &ends, &ended);
                None
            }
        };
        self.entries.push_back(Entry { slot, cancel, _task: task });
        after.unwrap_or(own)
    }

    /// The number of the job that runs this order, if one does.
    fn running(&self, order: &Order) -> Option<u64> {
        self.entries.iter().find_map(|entry| {
            let job = lock(&entry.slot);
            let same = job.order.target == order.target && job.order.operation.same_work(&order.operation);
            (!job.report.state.is_final() && same).then_some(job.number)
        })
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
    ended: Ended,
}

impl Progress for Shown {
    fn show(&mut self, report: &Report) {
        {
            let mut view = lock(&self.slot);
            *view = Arc::new(JobView { report: report.clone(), ..JobView::clone(&view) });
        }
        if report.state.is_final() {
            self.ended.send_replace(true);
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
            end_now(&self.slot, state, &self.ends, &self.ended);
        }
    }
}

fn start(
    tasks: &Tasks,
    order: Order,
    (parts, access): (Parts, AccessHandle),
    (slot, ends, ended): (&Slot, Ends, Ended),
) -> TaskHandle {
    let name = TaskName::scoped("job", order.target.cluster.as_str());
    let slot = Arc::clone(slot);
    tasks.start(name, Restart::Never, move || {
        let (order, parts, access) = (order.clone(), parts.clone(), access.clone());
        let mut shown = Shown { slot: Arc::clone(&slot), ends: ends.clone(), ended: Arc::clone(&ended) };
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

fn end_now(slot: &Slot, state: JobState, ends: &Ends, ended: &Ended) {
    let view = {
        let mut view = lock(slot);
        let mut report = view.report.clone();
        report.state = state;
        *view = Arc::new(JobView { report, ..JobView::clone(&view) });
        Arc::clone(&view)
    };
    ended.send_replace(true);
    ends.record(&view);
}

impl Inner {
    /// Starts a job for each order: one for an action, one for each row of a bulk action.
    /// With `in_turn`, each job waits for the end of the one before it: the documents of a
    /// text go in their sequence, as kubectl sends them.
    pub(super) fn run_orders(&self, orders: Vec<Order>, in_turn: bool) {
        let ends = Ends { journal: self.journal.clone(), repaint: self.repaint.clone() };
        let mut state = self.state();
        if state.phase == Phase::Stopped {
            return;
        }
        let mut after = None;
        for order in orders {
            let (cancel, signal) = Cancel::pair();
            let id = &order.target.cluster;
            // A read-only cluster gets no write, also from a view that did not close its gate.
            let refusal = state.config.refusal(id).filter(|_| order.operation.writes());
            let cluster = match (refusal, state.connected.get(id)) {
                (Some(refusal), _) => Err(CoreError::new(ErrorClass::Forbidden, refusal)),
                (None, Some(connected)) => Ok(connected.job_parts(signal, self.downloads.clone())),
                (None, None) => Err(CoreError::invalid(format!("{id} is not connected"))),
            };
            let next = state.jobs.start(&self.tasks, (order, cluster), cancel, ends.clone(), after.take());
            after = in_turn.then_some(next);
        }
        drop(state);
        self.repaint.request();
    }
}

#[cfg(test)]
mod tests;
