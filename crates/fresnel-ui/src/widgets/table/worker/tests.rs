//! The worker thread with a slow computation: each test holds a computation at a gate
//! and changes the mailbox in that time. No test waits a fixed time for a result.

use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::ThreadId;

use fresnel_core::net::task::End;

use super::*;
use crate::testing::{LIMIT, QUIET, TestTasks};

fn id(query: u64, data: u64) -> RequestId {
    RequestId { query, data }
}

/// A worker whose computation tells its start and then waits for the test.
struct Gated {
    worker: Worker<u64, u64>,
    /// The request of each computation that started.
    started: Receiver<u64>,
    /// One message lets one computation end.
    release: Sender<()>,
    /// One message for each result in the mailbox.
    woken: Receiver<()>,
    bed: TestTasks,
}

impl Gated {
    fn start() -> Self {
        let bed = TestTasks::new();
        let ((started_at, started), (release, gate), (wake, woken)) = (channel(), channel::<()>(), channel());
        let compute = move |request: &u64, cancel: &Cancel<'_>| {
            started_at.send(*request).expect("the test reads each start");
            gate.recv().expect("the test opens the gate");
            (!cancel.is_set()).then_some(*request * 2)
        };
        let wake: Wake = Arc::new(move || wake.send(()).expect("the test reads each wake"));
        let worker = Worker::start(&bed.tasks, TaskName::scoped("table rows", "test"), wake, compute);
        Self { worker, started, release, woken, bed }
    }

    fn started(&self) -> u64 {
        self.started.recv_timeout(LIMIT).expect("a computation starts")
    }

    /// Lets the computation in progress end and gives its result.
    fn result(&self) -> (RequestId, u64) {
        self.release.send(()).expect("the thread waits at the gate");
        self.woken.recv_timeout(LIMIT).expect("the thread wakes the UI for a result");
        let done = self.worker.take().expect("a result after a wake");
        (done.id, done.output)
    }
}

/// Defect A1 of the prototype: each new snapshot stopped the computation in progress, so
/// a stream of snapshots faster than the computation gave no rows at all.
#[test]
fn new_data_does_not_stop_a_computation_and_the_newest_request_is_the_next_one() {
    let gated = Gated::start();
    gated.worker.submit(id(1, 1), 10);
    assert_eq!(gated.started(), 10);
    // Three snapshots arrive while the rows of the first one are computed.
    for data in 2..=4 {
        gated.worker.submit(id(1, data), data * 10);
    }
    assert_eq!(gated.result(), (id(1, 1), 20), "the computation in progress gave its rows");
    assert_eq!(gated.started(), 40, "the mailbox has one place: the newest request is the next one");
    assert_eq!(gated.result(), (id(1, 4), 80));
    assert!(gated.started.recv_timeout(QUIET).is_err(), "no request waits");
    assert!(gated.worker.take().is_none() && gated.worker.failure().is_none());
    assert!(gated.bed.reports.try_recv().is_err(), "no run of the thread ended");
}

#[test]
fn a_new_query_stops_the_computation_in_progress_and_its_result_never_shows() {
    let gated = Gated::start();
    gated.worker.submit(id(1, 1), 10);
    assert_eq!(gated.started(), 10);
    gated.worker.submit(id(2, 1), 50);
    // The first computation finds its cancel mark and gives no rows.
    gated.release.send(()).expect("the thread waits at the gate");
    assert_eq!(gated.started(), 50);
    assert!(gated.worker.take().is_none(), "a stopped computation leaves no result");
    assert_eq!(gated.result(), (id(2, 1), 100));
    assert!(gated.woken.try_recv().is_err(), "one wake, for the one result");
}

/// Defect A3 of the prototype: after a panic of the thread, the table showed "updating"
/// with no end, and nothing told why.
#[test]
fn after_a_panic_the_thread_tells_why_and_then_computes_the_request_that_waits() {
    let bed = TestTasks::new();
    let broken = Arc::new(AtomicBool::new(true));
    let (wake, woken) = channel();
    let compute = {
        let broken = Arc::clone(&broken);
        move |request: &u64, _cancel: &Cancel<'_>| {
            assert!(!broken.load(Ordering::Relaxed), "no rows for {request}");
            Some(*request * 2)
        }
    };
    let wake: Wake = Arc::new(move || wake.send(()).expect("the test reads each wake"));
    let worker = Worker::start(&bed.tasks, TaskName::scoped("table rows", "test"), wake, compute);
    assert_eq!(worker.failure(), None, "the thread runs");
    worker.submit(id(1, 1), 21);

    let report = bed.reports.recv_timeout(LIMIT).expect("the report of the run that ended");
    assert_eq!(report.task.to_string(), "table rows [test]");
    assert_eq!(report.end, End::Panicked("no rows for 21".to_owned()));
    assert!(report.restart_in.is_some(), "the thread starts again");
    // The thread fails at each start, so some read finds it in its pause.
    let failure = (0..).find_map(|_| worker.failure()).expect("the state of the pause");
    assert_eq!(failure, "panicked: no rows for 21; it starts again");
    assert!(worker.take().is_none());

    // The defect is gone. No new request arrives: the one of the panic stayed in the mailbox.
    broken.store(false, Ordering::Relaxed);
    woken.recv_timeout(LIMIT).expect("the next run computes the request");
    let done = worker.take().expect("a result");
    assert_eq!((done.id, done.output), (id(1, 1), 42));
}

/// A result that tells the thread of its drop.
struct Tracked(Sender<ThreadId>);

impl Drop for Tracked {
    fn drop(&mut self) {
        self.0.send(std::thread::current().id()).expect("the test reads each drop");
    }
}

/// Plan section 7.3: the UI thread frees no rows. Rows can hold the last pointer to
/// each object of a store.
#[test]
fn the_thread_frees_the_results_that_the_table_gave_back_or_did_not_take() {
    let bed = TestTasks::new();
    let ((dropped_at, dropped), (wake, woken)) = (channel(), channel());
    let compute = move |_request: &u64, _cancel: &Cancel<'_>| Some(Tracked(dropped_at.clone()));
    let wake: Wake = Arc::new(move || wake.send(()).expect("the test reads each wake"));
    let worker = Worker::start(&bed.tasks, TaskName::scoped("table rows", "test"), wake, compute);
    let here = std::thread::current().id();

    worker.submit(id(1, 1), 1);
    woken.recv_timeout(LIMIT).expect("the first result");
    let shown = worker.take().expect("a result").output;
    // The second result waits in the mailbox, and the third one takes its place.
    worker.submit(id(1, 2), 2);
    woken.recv_timeout(LIMIT).expect("the second result");
    worker.submit(id(1, 3), 3);
    woken.recv_timeout(LIMIT).expect("the third result");
    assert_ne!(dropped.recv_timeout(LIMIT).expect("the drop of the second result"), here);

    worker.discard(shown);
    assert_ne!(dropped.recv_timeout(LIMIT).expect("the drop of the first result"), here);
    assert!(dropped.try_recv().is_err(), "the third result is in the mailbox");
}

/// Tells the test that the thread ended: the thread owns the computation, and with it
/// this value.
struct Ended(Sender<()>);

impl Drop for Ended {
    fn drop(&mut self) {
        self.0.send(()).expect("the test waits for the end");
    }
}

#[test]
fn the_drop_of_the_owner_ends_the_thread_and_that_is_no_fault() {
    let bed = TestTasks::new();
    let (ended_at, ended) = channel();
    let ended_at = Ended(ended_at);
    let compute = move |request: &u64, _cancel: &Cancel<'_>| {
        let _held = &ended_at;
        Some(*request)
    };
    let worker = Worker::start(&bed.tasks, TaskName::scoped("table rows", "test"), Arc::new(|| {}), compute);
    worker.submit(id(1, 1), 1);
    let running = || bed.tasks.snapshot().iter().filter(|task| task.name.to_string() == "table rows [test]").count();
    assert_eq!(running(), 1);

    drop(worker);
    ended.recv_timeout(LIMIT).expect("the thread ends");
    assert_eq!(running(), 0, "the state of the task went with its owner");
    assert!(bed.reports.recv_timeout(QUIET).is_err(), "a stop is no fault, so it has no report");
}

/// Plan section 7.3: a table that is not painted holds no rows. Its owner empties the
/// mailbox, and the result of a computation in progress has no reader then.
#[test]
fn an_empty_mailbox_takes_no_result_and_the_thread_frees_each_one() {
    let bed = TestTasks::new();
    let ((started_at, started), (release, gate)) = (channel(), channel::<()>());
    let ((dropped_at, dropped), (wake, woken)) = (channel(), channel());
    let compute = move |request: &u64, _cancel: &Cancel<'_>| {
        started_at.send(*request).expect("the test reads each start");
        gate.recv().expect("the test opens the gate");
        Some(Tracked(dropped_at.clone()))
    };
    let wake: Wake = Arc::new(move || wake.send(()).expect("the test reads each wake"));
    let worker = Worker::start(&bed.tasks, TaskName::scoped("table rows", "test"), wake, compute);
    let here = std::thread::current().id();
    let next = |limit| started.recv_timeout(limit);

    // One result waits in the mailbox, and the computation of a second one is in progress.
    worker.submit(id(1, 1), 1);
    assert_eq!(next(LIMIT), Ok(1));
    release.send(()).expect("the thread waits at the gate");
    woken.recv_timeout(LIMIT).expect("the first result");
    worker.submit(id(1, 2), 2);
    assert_eq!(next(LIMIT), Ok(2));

    worker.clear();
    assert!(worker.take().is_none(), "the result that waited left the mailbox");
    release.send(()).expect("the thread waits at the gate");
    for result in ["the one with no reader", "the one that waited"] {
        assert_ne!(dropped.recv_timeout(LIMIT).expect(result), here, "{result}");
    }
    assert!(woken.recv_timeout(QUIET).is_err() && worker.take().is_none(), "no wake for a result with no reader");
    assert!(next(QUIET).is_err(), "the request left the mailbox too");

    worker.submit(id(1, 3), 3);
    assert_eq!(next(LIMIT), Ok(3), "the thread computes the next request as before");
}
