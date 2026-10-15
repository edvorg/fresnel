use fresnel_net::WallClock;
use fresnel_net::task::TaskReport;
use tokio::runtime::Handle;

use super::*;
use crate::cluster::ClusterId;
use crate::kinds::Kind;
use crate::ops::drain::DrainOptions;
use crate::ops::{Operation, Target};
use crate::store::ObjectKey;

/// The cluster of a job that is not connected.
fn not_connected() -> Result<ClusterParts, CoreError> {
    Err(CoreError::invalid("a is not connected"))
}

fn order(operation: Operation) -> Order {
    let target = Target {
        cluster: ClusterId::new("a"),
        kind: Kind::Deployment,
        key: ObjectKey::new("shop", "web"),
        uid: "u".into(),
    };
    Order::new(target, operation).expect("an order")
}

fn ends() -> Ends {
    Ends { journal: Journal::new(100, WallClock::system()), repaint: Repaint::new() }
}

fn states(jobs: &Jobs) -> Vec<(u64, JobState)> {
    jobs.view().jobs.iter().map(|job| (job.number, job.report.state.clone())).collect()
}

#[tokio::test]
async fn a_job_for_a_cluster_that_is_not_connected_ends_at_once_as_not_sent_with_a_journal_entry() {
    let tasks = Tasks::new(Handle::current(), |report: TaskReport| panic!("no task: {report}"));
    let (mut jobs, ends) = (Jobs::default(), ends());
    jobs.start(&tasks, order(Operation::Scale(3)), not_connected(), Cancel::pair().0, ends.clone());
    let error = CoreError::invalid("a is not connected");
    assert_eq!(states(&jobs), [(1, JobState::NotSent(error))]);
    let entries = ends.journal.view();
    let texts: Vec<&str> = entries.entries.iter().map(|entry| entry.text.as_str()).collect();
    assert_eq!(texts, ["Scale to 3 of Deployment shop/web: not sent: a is not connected"]);
}

#[tokio::test]
async fn a_full_list_drops_its_oldest_ended_job_and_takes_no_job_when_each_one_runs() {
    let tasks = Tasks::new(Handle::current(), |report: TaskReport| panic!("no task: {report}"));
    let (mut jobs, ends) = (Jobs::default(), ends());
    for _ in 0..JOB_LIMIT {
        jobs.start(&tasks, order(Operation::Restart), not_connected(), Cancel::pair().0, ends.clone());
    }
    let running = Arc::new(JobView { number: 9999, order: order(Operation::RolloutStatus), report: Report::default() });
    let entry = |job: &Arc<JobView>| Entry {
        slot: Arc::new(Mutex::new(Arc::clone(job))),
        cancel: Cancel::pair().0,
        _task: None,
    };
    jobs.entries[3] = entry(&running);
    jobs.start(&tasks, order(Operation::Restart), not_connected(), Cancel::pair().0, ends.clone());
    let view = jobs.view();
    assert_eq!((view.jobs.len(), view.dropped), (JOB_LIMIT, 1));
    assert_eq!(view.jobs[0].number, 2, "the oldest job that ended left");
    assert_eq!(view.jobs[2].number, 9999, "a running job stays");
    assert_eq!(view.room(), JOB_LIMIT - 1, "each job that ended makes room");

    let mut full = Jobs { entries: (0..JOB_LIMIT).map(|_| entry(&running)).collect(), ..Jobs::default() };
    assert_eq!(full.view().room(), 0);
    full.start(&tasks, order(Operation::Restart), not_connected(), Cancel::pair().0, ends.clone());
    let view = full.view();
    assert_eq!((view.jobs.len(), view.dropped), (JOB_LIMIT, 0), "the list does not grow over its limit");
    let last = ends.journal.view().entries.last().map(|entry| entry.text.clone());
    let text = format!("Restart of Deployment shop/web: not sent: {JOB_LIMIT} jobs run: the job waits for none");
    assert_eq!(last, Some(text));
}

#[tokio::test]
async fn the_same_order_while_its_job_runs_ends_as_not_sent_and_an_other_order_goes_on() {
    let tasks = Tasks::new(Handle::current(), |report: TaskReport| panic!("no task: {report}"));
    let (mut jobs, ends) = (Jobs::default(), ends());
    let running = Arc::new(JobView { number: 1, order: order(Operation::Rollback(0)), report: Report::default() });
    jobs.entries.push_back(Entry { slot: Arc::new(Mutex::new(running)), cancel: Cancel::pair().0, _task: None });
    jobs.number = 1;

    jobs.start(&tasks, order(Operation::Rollback(0)), not_connected(), Cancel::pair().0, ends.clone());
    jobs.start(&tasks, order(Operation::Rollback(2)), not_connected(), Cancel::pair().0, ends.clone());
    let same = CoreError::invalid("job 1 runs the same operation on the object");
    let other = CoreError::invalid("a is not connected");
    assert_eq!(
        states(&jobs),
        [(1, JobState::Pending), (2, JobState::NotSent(same)), (3, JobState::NotSent(other.clone()))]
    );

    // A second drain of a node with other options would race the first one for its pods.
    let drain = |force| {
        let node = Target { kind: Kind::Node, key: ObjectKey::new("", "pool-a1"), ..order(Operation::Restart).target };
        Order::new(node, Operation::Drain(DrainOptions { force, ..DrainOptions::default() })).expect("an order")
    };
    let running = Arc::new(JobView { number: 4, order: drain(false), report: Report::default() });
    jobs.entries.push_back(Entry { slot: Arc::new(Mutex::new(running)), cancel: Cancel::pair().0, _task: None });
    jobs.number = 4;
    jobs.start(&tasks, drain(true), not_connected(), Cancel::pair().0, ends.clone());
    let same = CoreError::invalid("job 4 runs the same operation on the object");
    assert_eq!(states(&jobs)[4], (5, JobState::NotSent(same)));

    // A job that ended does not hold its order.
    let mut done = Jobs::default();
    let ended = Report { state: JobState::Done, ..Report::default() };
    let job = Arc::new(JobView { number: 1, order: order(Operation::Rollback(0)), report: ended });
    done.entries.push_back(Entry { slot: Arc::new(Mutex::new(job)), cancel: Cancel::pair().0, _task: None });
    done.number = 1;
    done.start(&tasks, order(Operation::Rollback(0)), not_connected(), Cancel::pair().0, ends.clone());
    assert_eq!(states(&done)[1], (2, JobState::NotSent(other)));
}

#[tokio::test]
async fn a_cancel_names_its_job_and_sets_its_signal_only() {
    let mut jobs = Jobs::default();
    let pending =
        |number| Arc::new(JobView { number, order: order(Operation::RolloutStatus), report: Report::default() });
    let mut signals = Vec::new();
    for number in [1, 2] {
        let (cancel, signal) = Cancel::pair();
        signals.push(signal);
        jobs.entries.push_back(Entry { slot: Arc::new(Mutex::new(pending(number))), cancel, _task: None });
    }
    jobs.cancel(2);
    jobs.cancel(7);
    let set: Vec<bool> = jobs.entries.iter().map(|entry| *entry.cancel.borrow()).collect();
    assert_eq!(set, [false, true]);
}

#[tokio::test]
async fn a_job_that_stops_before_its_end_has_an_unknown_result_when_it_can_have_written() {
    for (operation, unknown) in [(Operation::Scale(2), true), (Operation::History, false)] {
        let slot: Slot =
            Arc::new(Mutex::new(Arc::new(JobView { number: 1, order: order(operation), report: Report::default() })));
        drop(Shown { slot: Arc::clone(&slot), ends: ends() });
        let state = lock(&slot).report.state.clone();
        let error = CoreError::invalid("the job stopped before its end");
        assert_eq!(state, if unknown { JobState::Unknown(error) } else { JobState::Failed(error) });
    }
    let done = Report { state: JobState::Done, ..Report::default() };
    let slot: Slot =
        Arc::new(Mutex::new(Arc::new(JobView { number: 1, order: order(Operation::Restart), report: done })));
    drop(Shown { slot: Arc::clone(&slot), ends: ends() });
    assert_eq!(lock(&slot).report.state, JobState::Done, "a final state stays");
}
