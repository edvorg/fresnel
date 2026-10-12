//! The task start function: each end of a run gives a state, a report and, if asked, a restart.
//!
//! Time is paused in each test. A pause of the backoff thus costs no real time, and a wait
//! for a signal that cannot arrive ends at once with a failure.

#![allow(clippy::expect_used, reason = "a test stops with a panic")]

use std::convert::Infallible;
use std::fmt::Display;
use std::future::{Ready, pending};
use std::pin::Pin;
use std::time::Duration;

use fresnel_net::task::{Backoff, End, Restart, TaskHandle, TaskName, TaskReport, TaskState, TaskStatus, Tasks};
use tokio::runtime::Handle;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::time::{Instant, timeout};

const MS: Duration = Duration::from_millis(1);
/// No jitter, so that a test can assert each pause.
const BACKOFF: Backoff = Backoff {
    first: Duration::from_millis(100),
    max: Duration::from_millis(500),
    jitter_percent: 0,
    reset_after: Duration::from_secs(10),
};

type Run = Pin<Box<dyn Future<Output = Result<(), Infallible>> + Send>>;

fn tasks_with_reports() -> (Tasks, UnboundedReceiver<TaskReport>) {
    let (sender, reports) = unbounded_channel();
    let reporter = move |report: TaskReport| sender.send(report).expect("the test holds the receiver");
    (Tasks::new(Handle::current(), reporter), reports)
}

/// Waits for a signal of a task. When no task can make progress, paused time jumps to the
/// limit. A supervisor that stopped thus fails the test and does not hang it.
async fn in_time<T>(signal: impl Future<Output = T>) -> T {
    timeout(Duration::from_secs(3_600), signal).await.expect("the signal arrives")
}

fn status(tasks: &Tasks, handle: &TaskHandle) -> TaskStatus {
    tasks.snapshot().into_iter().find(|status| status.id == handle.id()).expect("a live handle has a status")
}

/// A body that sends the number of each run and then stays alive. If asked, run 1 panics.
fn counted_runs(runs: UnboundedSender<u32>, first_run_panics: bool) -> impl FnMut() -> Run + Send {
    let mut run = 0;
    move || {
        run += 1;
        let (run, runs) = (run, runs.clone());
        Box::pin(async move {
            runs.send(run).expect("the test holds the receiver");
            if first_run_panics && run == 1 {
                panic!("first run fails");
            }
            pending().await
        })
    }
}

#[tokio::test(start_paused = true)]
async fn a_panic_gives_a_state_a_report_and_a_restart() {
    let (tasks, mut reports) = tasks_with_reports();
    let (run_sender, mut runs) = unbounded_channel();
    let name = TaskName::scoped("watch", "cluster-a");
    let handle = tasks.start(name, Restart::Always(BACKOFF), counted_runs(run_sender, true));

    let report = in_time(reports.recv()).await.expect("a report");
    let panicked = End::Panicked("first run fails".to_owned());
    assert_eq!((&report.end, report.restart_in), (&panicked, Some(100 * MS)));
    assert_eq!(report.to_string(), "task watch [cluster-a] panicked: first run fails; restart in 100 ms");
    let backoff = TaskState::Backoff { last: panicked, early_ends: 1, pause: 100 * MS };
    assert_eq!(status(&tasks, &handle).state, backoff);

    assert_eq!((in_time(runs.recv()).await, in_time(runs.recv()).await), (Some(1), Some(2)));
    let status = status(&tasks, &handle);
    assert_eq!((status.state, status.runs), (TaskState::Running, 2));
    assert!(reports.try_recv().is_err(), "a run that is alive gives no report");
}

#[tokio::test(start_paused = true)]
async fn each_early_end_makes_the_pause_longer_up_to_the_limit() {
    let (tasks, mut reports) = tasks_with_reports();
    let start = Instant::now();
    let _handle = tasks.start(TaskName::new("sampler"), Restart::Always(BACKOFF), || async { Err("no route") });

    let mut pauses = Vec::new();
    for _ in 0..5 {
        let report = in_time(reports.recv()).await.expect("a report");
        assert_eq!(report.end, End::Failed("no route".to_owned()));
        pauses.push(report.restart_in.expect("a restart"));
    }
    assert_eq!(pauses, [100, 200, 400, 500, 500].map(|ms| ms * MS));
    assert_eq!(start.elapsed(), 1_200 * MS, "the supervisor waits for each pause before the next run");
}

#[tokio::test(start_paused = true)]
async fn a_long_run_resets_the_pause() {
    let (tasks, mut reports) = tasks_with_reports();
    let mut run = 0;
    let _handle = tasks.start(TaskName::new("sampler"), Restart::Always(BACKOFF), move || {
        run += 1;
        async move {
            if run == 3 {
                tokio::time::sleep(BACKOFF.reset_after).await;
            }
            Ok::<(), Infallible>(())
        }
    });

    let mut pauses = Vec::new();
    for _ in 0..4 {
        let report = in_time(reports.recv()).await.expect("a report");
        assert_eq!(report.end, End::Done, "a task that restarts must not end, so `Ok` gets a report too");
        pauses.push(report.restart_in.expect("a restart"));
    }
    assert_eq!(pauses, [100, 200, 100, 200].map(|ms| ms * MS));
}

/// Runs a task one time and gives its report and its final status.
async fn run_one_time<F, E>(body: impl FnMut() -> F + Send + 'static) -> (TaskReport, TaskStatus)
where
    F: Future<Output = Result<(), E>> + Send + 'static,
    E: Display + 'static,
{
    let (tasks, mut reports) = tasks_with_reports();
    let handle = tasks.start(TaskName::new("job"), Restart::Never, body);
    let report = in_time(reports.recv()).await.expect("a report");
    assert!(reports.try_recv().is_err(), "one run gives one report");
    (report, status(&tasks, &handle))
}

#[tokio::test(start_paused = true)]
async fn a_task_that_runs_one_time_keeps_its_end_as_the_final_state() {
    let done = run_one_time(|| async { Ok::<(), Infallible>(()) }).await;
    let failed = run_one_time(|| async { Err("refused") }).await;
    let panicked = run_one_time(|| async { panic!("code {}", 7) as Result<(), Infallible> }).await;
    let no_future = run_one_time(|| -> Ready<Result<(), Infallible>> { panic!("no future") }).await;
    let no_text = run_one_time(|| async { std::panic::panic_any(7_u8) as Result<(), Infallible> }).await;

    let ends = [
        (done, End::Done, "task job ended; no restart"),
        (failed, End::Failed("refused".to_owned()), "task job failed: refused; no restart"),
        (panicked, End::Panicked("code 7".to_owned()), "task job panicked: code 7; no restart"),
        (no_future, End::Panicked("no future".to_owned()), "task job panicked: no future; no restart"),
        (
            no_text,
            End::Panicked("a panic without a text".to_owned()),
            "task job panicked: a panic without a text; no restart",
        ),
    ];
    for ((report, status), end, text) in ends {
        assert_eq!((&report.end, report.restart_in, report.to_string().as_str()), (&end, None, text));
        assert_eq!((status.state, status.runs), (TaskState::Ended(end), 1));
    }
}

#[tokio::test(start_paused = true)]
async fn a_dropped_handle_stops_the_task_and_removes_its_state() {
    let (tasks, mut reports) = tasks_with_reports();
    let (run_sender, mut runs) = unbounded_channel();
    let handle = tasks.start(TaskName::new("forward"), Restart::Always(BACKOFF), counted_runs(run_sender, false));
    assert_eq!(in_time(runs.recv()).await, Some(1));
    assert_eq!(tasks.snapshot().len(), 1);

    drop(handle);
    assert_eq!(in_time(runs.recv()).await, None, "the body and its run are gone");
    assert!(tasks.snapshot().is_empty());
    assert!(reports.try_recv().is_err(), "a stop by the owner is not a fault");
}

#[tokio::test(start_paused = true)]
async fn a_reporter_that_panics_does_not_stop_the_task() {
    let tasks = Tasks::new(Handle::current(), |_: TaskReport| panic!("the journal is broken"));
    let (run_sender, mut runs) = unbounded_channel();
    let handle = tasks.start(TaskName::new("watch"), Restart::Always(BACKOFF), counted_runs(run_sender, true));

    assert_eq!((in_time(runs.recv()).await, in_time(runs.recv()).await), (Some(1), Some(2)));
    assert_eq!(status(&tasks, &handle).state, TaskState::Running);
    assert_eq!(tasks.lost_reports(), 1);
}

#[test]
fn a_thread_with_no_runtime_starts_a_task_and_a_stopped_runtime_gives_a_final_state() {
    let runtime =
        tokio::runtime::Builder::new_current_thread().enable_time().start_paused(true).build().expect("a runtime");
    let tasks = Tasks::new(runtime.handle().clone(), |_: TaskReport| {});
    let (run_sender, mut runs) = unbounded_channel();
    let handle = tasks.start(TaskName::new("sampler"), Restart::Always(BACKOFF), counted_runs(run_sender, false));
    assert_eq!(runtime.block_on(in_time(runs.recv())), Some(1));
    assert_eq!(status(&tasks, &handle).state, TaskState::Running);

    drop(runtime);
    assert_eq!(status(&tasks, &handle).state, TaskState::Ended(End::Stopped));
}
