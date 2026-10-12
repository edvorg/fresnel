//! A second fault while the start function handles the first one.

use std::convert::Infallible;
use std::future::pending;
use std::sync::atomic::{AtomicU32, Ordering};

use fresnel_net::task::{Restart, TaskName, TaskReport, TaskState, Tasks};
use tokio::runtime::Handle;
use tokio::sync::mpsc::unbounded_channel;

use super::{BACKOFF, MS, in_time, status, tasks_with_reports};

#[tokio::test(start_paused = true)]
async fn faults_during_the_restart_do_not_stop_the_task() {
    let (sender, mut reports) = unbounded_channel();
    let seen = AtomicU32::new(0);
    let tasks = Tasks::new(Handle::current(), move |report: TaskReport| {
        assert_ne!(seen.fetch_add(1, Ordering::Relaxed), 1, "the journal fails for the second report");
        sender.send(report).expect("the test holds the receiver");
    });
    let (alive_sender, mut alive) = unbounded_channel();
    let mut run = 0;
    let handle = tasks.start(TaskName::new("watch"), Restart::Always(BACKOFF), move || {
        run += 1;
        assert!(run > 3, "run {run} cannot start");
        alive_sender.send(run).expect("the test holds the receiver");
        pending::<Result<(), Infallible>>()
    });

    assert_eq!(in_time(alive.recv()).await, Some(4));
    let status = status(&tasks, &handle);
    assert_eq!((status.state, status.runs), (TaskState::Running, 4));
    let pauses = [reports.try_recv(), reports.try_recv()].map(|report| report.expect("a report").restart_in);
    assert_eq!(pauses, [Some(100 * MS), Some(400 * MS)], "the pause grows also when its report is lost");
    assert_eq!(tasks.lost_reports(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_handle_that_drops_during_the_pause_stops_the_restart() {
    let (tasks, mut reports) = tasks_with_reports();
    let (run_sender, mut runs) = unbounded_channel();
    let handle = tasks.start(TaskName::new("forward"), Restart::Always(BACKOFF), move || {
        run_sender.send(()).expect("the test holds the receiver");
        async { Err("no route") }
    });
    let report = in_time(reports.recv()).await.expect("a report");
    assert_eq!(report.restart_in, Some(100 * MS), "the task is in its pause");

    drop(handle);
    let runs = (in_time(runs.recv()).await, in_time(runs.recv()).await);
    assert_eq!(runs, (Some(()), None), "one run, then the body is gone");
    assert!(tasks.snapshot().is_empty());
    assert!(reports.try_recv().is_err(), "a stop by the owner is not a fault");
}
