//! The start function on worker threads, in real time.

use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use fresnel_net::task::{End, Restart, TaskName, TaskReport, TaskState, Tasks};
use tokio::runtime::{Builder, Runtime};

use super::{BACKOFF, MS, status};

/// The limit of each wait. A signal that does not arrive fails the test and does not hang it.
const LIMIT: Duration = Duration::from_secs(30);

fn tasks_with_reports(runtime: &Runtime) -> (Tasks, Receiver<TaskReport>) {
    let (sender, reports) = channel();
    let reporter = move |report: TaskReport| sender.send(report).expect("the test holds the receiver");
    (Tasks::new(runtime.handle().clone(), reporter), reports)
}

#[test]
fn a_fault_of_the_supervisor_itself_gives_a_final_state_and_a_report() {
    // This runtime has no timer. Thus the pause before the restart panics in the supervisor.
    let runtime = Builder::new_multi_thread().worker_threads(1).build().expect("a runtime");
    let (tasks, reports) = tasks_with_reports(&runtime);
    let handle = tasks.start(TaskName::new("sampler"), Restart::Always(BACKOFF), || async { Err("no route") });

    let run = reports.recv_timeout(LIMIT).expect("the report of the run");
    assert_eq!((run.end, run.restart_in), (End::Failed("no route".to_owned()), Some(100 * MS)));
    let fault = reports.recv_timeout(LIMIT).expect("the report of the fault");
    assert!(matches!(fault.end, End::Panicked(_)) && fault.restart_in.is_none(), "{fault}");
    assert_eq!(status(&tasks, &handle).state, TaskState::Ended(fault.end));
}
