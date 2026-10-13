//! The start function for a thread: a worker has the same state, report and restart as a
//! task of the runtime (design rule 9). These tests use real threads and real time.

use std::convert::Infallible;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use fresnel_net::task::{Backoff, End, Restart, Stop, TaskHandle, TaskName, TaskReport, TaskState, Tasks};
use tokio::runtime::{Builder, Runtime};

use super::{MS, status};

/// The limit of each wait. A signal that does not arrive fails the test and does not hang it.
const LIMIT: Duration = Duration::from_secs(30);
/// Short pauses with no jitter, because these tests wait in real time.
const FAST: Backoff = Backoff { first: MS, max: Duration::from_millis(4), jitter_percent: 0, reset_after: LIMIT };
/// A pause that no test waits for: it is longer than the limit of a wait.
const PAUSE: Duration = Duration::from_secs(600);
const LONG_PAUSE: Backoff = Backoff { first: PAUSE, max: PAUSE, jitter_percent: 0, reset_after: LIMIT };

fn tasks_with_reports() -> (Runtime, Tasks, Receiver<TaskReport>) {
    let runtime = Builder::new_current_thread().build().expect("a runtime");
    let (sender, reports) = channel();
    let reporter = move |report: TaskReport| sender.send(report).expect("the test holds the receiver");
    let tasks = Tasks::new(runtime.handle().clone(), reporter);
    (runtime, tasks, reports)
}

/// Sends one signal at its drop. The body of a task owns it, so the signal tells that the
/// thread of the task dropped its body and is at its end.
struct BodyGone(Sender<()>);

impl Drop for BodyGone {
    fn drop(&mut self) {
        self.0.send(()).expect("the test holds the receiver");
    }
}

/// One result of the worker: the number, and the name of the thread that computed it.
type Doubled = (u32, Option<String>);

/// A worker as the UI builds it: it doubles each request. The handle is the first field,
/// so it drops before the input of the body, and the end of the input is not a fault.
struct Doubler {
    task: TaskHandle,
    requests: Sender<u32>,
    results: Receiver<Doubled>,
    gone: Receiver<()>,
}

impl Doubler {
    /// Starts the worker. The request 13 makes its thread panic.
    fn start(tasks: &Tasks, restart: Restart) -> Self {
        let (requests, inbox) = channel::<u32>();
        let (outbox, results) = channel();
        let (gone_sender, gone) = channel();
        let body_gone = BodyGone(gone_sender);
        let task = tasks.start_thread(TaskName::scoped("rows", "pods"), restart, move |_stop: &Stop| {
            let _owned_by_the_body = &body_gone;
            // The loop ends when the owner drops the sender of the requests.
            while let Ok(request) = inbox.recv() {
                if request == 13 {
                    panic!("request {request} has no result");
                }
                let thread = std::thread::current().name().map(str::to_owned);
                outbox.send((request * 2, thread)).expect("the test holds the receiver");
            }
            Ok::<(), Infallible>(())
        });
        Self { task, requests, results, gone }
    }

    fn double(&self, request: u32) -> Doubled {
        self.requests.send(request).expect("the worker takes requests");
        self.results.recv_timeout(LIMIT).expect("a result in time")
    }
}

#[test]
fn a_worker_runs_on_a_thread_with_the_name_of_its_task_and_needs_no_runtime() {
    let (runtime, tasks, reports) = tasks_with_reports();
    drop(runtime);
    let worker = Doubler::start(&tasks, Restart::Always(FAST));
    assert_eq!(worker.double(4), (8, Some("rows [pods]".to_owned())));
    assert_ne!(std::thread::current().name(), Some("rows [pods]"));
    let status = status(&tasks, &worker.task);
    assert_eq!((status.state, status.runs, status.name.to_string().as_str()), (TaskState::Running, 1, "rows [pods]"));
    assert!(reports.try_recv().is_err(), "a run that is alive gives no report");
}

#[test]
fn a_panic_on_the_thread_gives_a_state_a_report_and_a_restart() {
    let (_runtime, tasks, reports) = tasks_with_reports();
    let worker = Doubler::start(&tasks, Restart::Always(FAST));
    assert_eq!(worker.double(1).0, 2);

    worker.requests.send(13).expect("the worker takes requests");
    let report = reports.recv_timeout(LIMIT).expect("a report");
    let panicked = End::Panicked("request 13 has no result".to_owned());
    assert_eq!((&report.end, report.restart_in), (&panicked, Some(MS)));
    assert_eq!(report.to_string(), "task rows [pods] panicked: request 13 has no result; restart in 1 ms");

    // The request after the fault gets its result from the next run.
    assert_eq!(worker.double(5).0, 10);
    let status = status(&tasks, &worker.task);
    assert_eq!((status.state, status.runs), (TaskState::Running, 2));
    assert!(reports.try_recv().is_err(), "one fault gives one report");
    assert_eq!(tasks.lost_reports(), 0);
}

#[test]
fn each_early_end_of_a_thread_makes_the_pause_longer_up_to_the_limit() {
    let (_runtime, tasks, reports) = tasks_with_reports();
    let service = tasks.start_thread(TaskName::new("sampler"), Restart::Always(FAST), |_: &Stop| Err("no data"));
    let mut pauses = Vec::new();
    for _ in 0..4 {
        let report = reports.recv_timeout(LIMIT).expect("a report");
        assert_eq!(report.end, End::Failed("no data".to_owned()));
        pauses.push(report.restart_in.expect("a restart"));
    }
    assert_eq!(pauses, [1, 2, 4, 4].map(|ms| ms * MS));
    assert!(status(&tasks, &service).runs >= 4);
}

/// The body of a job of the next test.
type Job = fn(&Stop) -> Result<(), &'static str>;

#[test]
fn a_thread_that_runs_one_time_keeps_its_end_as_the_final_state() {
    let (_runtime, tasks, reports) = tasks_with_reports();
    let ends: [(Job, End); 3] = [
        (|_| Ok(()), End::Done),
        (|_| Err("refused"), End::Failed("refused".to_owned())),
        (|_| panic!("code {}", 7), End::Panicked("code 7".to_owned())),
    ];
    for (body, end) in ends {
        let job = tasks.start_thread(TaskName::new("job"), Restart::Never, body);
        let report = reports.recv_timeout(LIMIT).expect("a report");
        assert_eq!((&report.end, report.restart_in), (&end, None));
        let status = status(&tasks, &job);
        assert_eq!((status.state, status.runs), (TaskState::Ended(end), 1));
        assert!(reports.try_recv().is_err(), "one run gives one report");
    }
}

#[test]
fn a_dropped_handle_ends_the_thread_and_removes_its_state_with_no_report() {
    let (_runtime, tasks, reports) = tasks_with_reports();
    // Many times: the thread must never see the end of its input before the stop.
    for _ in 0..200 {
        let worker = Doubler::start(&tasks, Restart::Always(LONG_PAUSE));
        assert_eq!(worker.double(2).0, 4);
        let Doubler { task, requests, results: _results, gone } = worker;
        drop(task);
        assert!(tasks.snapshot().is_empty(), "the state goes with the handle");
        drop(requests);
        gone.recv_timeout(LIMIT).expect("the thread ends when its input ends");
    }
    assert!(reports.try_recv().is_err(), "a stop by the owner is not a fault");
}

#[test]
fn a_stop_ends_the_pause_before_a_restart_and_a_body_that_computes() {
    let (_runtime, tasks, reports) = tasks_with_reports();
    let (gone_sender, gone) = channel();
    let body_gone = BodyGone(gone_sender.clone());
    let failing = tasks.start_thread(TaskName::new("sampler"), Restart::Always(LONG_PAUSE), move |_: &Stop| {
        let _owned_by_the_body = &body_gone;
        Err("no data")
    });
    assert_eq!(reports.recv_timeout(LIMIT).expect("a report").restart_in, Some(PAUSE), "the task is in its pause");
    drop(failing);
    gone.recv_timeout(LIMIT).expect("the thread does not wait for the end of the pause");

    // A body with no input reads the stop between two steps.
    let (step_sender, steps) = channel();
    let body_gone = BodyGone(gone_sender);
    let computing = tasks.start_thread(TaskName::new("sort"), Restart::Always(LONG_PAUSE), move |stop: &Stop| {
        let _owned_by_the_body = &body_gone;
        step_sender.send(()).expect("the test holds the receiver");
        while !stop.is_set() {
            std::thread::yield_now();
        }
        Ok::<(), Infallible>(())
    });
    steps.recv_timeout(LIMIT).expect("the body computes");
    drop(computing);
    gone.recv_timeout(LIMIT).expect("the body ends after the stop");
    assert!(tasks.snapshot().is_empty());
    assert!(reports.try_recv().is_err(), "a stop by the owner is not a fault");
}
