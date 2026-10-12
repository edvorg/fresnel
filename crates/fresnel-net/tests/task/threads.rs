//! The start function on worker threads, in real time: a fault of the supervisor itself, a slow
//! reporter, and many tasks that fail and stop while other threads read the states.

use std::future::pending;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use fresnel_net::task::{Backoff, End, Restart, TaskHandle, TaskName, TaskReport, TaskState, TaskStatus, Tasks};
use tokio::runtime::{Builder, Runtime};

use super::{BACKOFF, MS, status};

/// The limit of each wait. A signal that does not arrive fails the test and does not hang it.
const LIMIT: Duration = Duration::from_secs(30);
const TASKS: usize = 200;
const FAULTS: u32 = 4;
/// Short pauses, because these tests wait in real time.
const FAST: Backoff = Backoff { first: MS, max: Duration::from_millis(4), jitter_percent: 50, reset_after: LIMIT };

fn runtime_with_workers(count: usize) -> Runtime {
    Builder::new_multi_thread().worker_threads(count).enable_time().build().expect("a runtime")
}

fn tasks_with_reports(runtime: &Runtime) -> (Tasks, Receiver<TaskReport>) {
    let (sender, reports) = channel();
    let reporter = move |report: TaskReport| sender.send(report).expect("the test holds the receiver");
    (Tasks::new(runtime.handle().clone(), reporter), reports)
}

/// Starts `TASKS` tasks. Each one fails `FAULTS` times, then sends one signal and stays alive.
fn start_tasks_that_fail_first(tasks: &Tasks) -> (Vec<TaskHandle>, Receiver<()>) {
    let (alive_sender, alive) = channel();
    let start = |_| {
        let (alive, mut run) = (alive_sender.clone(), 0);
        tasks.start(TaskName::new("watch"), Restart::Always(FAST), move || {
            run += 1;
            let alive = alive.clone();
            async move {
                match run {
                    run if run > FAULTS => alive.send(()).expect("the test holds the receiver"),
                    run if run % 2 == 0 => return Err("no route"),
                    run => panic!("run {run} fails"),
                }
                pending::<Result<(), &str>>().await
            }
        })
    };
    ((0..TASKS).map(start).collect(), alive)
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

/// Starts a task that fails. Gives its report, and the states that a reader on a different
/// thread gets while the reporter still holds that report.
fn states_during_a_slow_report(restart: Restart) -> (TaskReport, Vec<TaskState>) {
    let runtime = runtime_with_workers(2);
    let (entered_sender, entered) = channel();
    let (release, released) = channel::<()>();
    let released = Mutex::new(released);
    let tasks = Tasks::new(runtime.handle().clone(), move |report: TaskReport| {
        entered_sender.send(report).expect("the test holds the receiver");
        released.lock().expect("no reporter panicked").recv().expect("the test releases the reporter");
    });
    let handle = tasks.start(TaskName::new("job"), restart, || async { Err("refused") });
    let report = entered.recv_timeout(LIMIT).expect("the reporter has the report");

    // The reader runs on a thread that the test can leave. A reader that waits thus fails the test.
    let reader = tasks.clone();
    let read = async { tokio::time::timeout(LIMIT, tokio::task::spawn_blocking(move || reader.snapshot())).await };
    let states = runtime.block_on(read);
    drop(handle);
    release.send(()).expect("the reporter waits for the release");
    let states = states.expect("the reader does not wait for the reporter").expect("the reader ends");
    (report, states.into_iter().map(|status| status.state).collect())
}

#[test]
fn a_reader_of_the_states_does_not_wait_for_a_slow_reporter_and_finds_the_new_state() {
    let (report, states) = states_during_a_slow_report(Restart::Never);
    assert_eq!(states, [TaskState::Ended(report.end)]);

    let (report, states) = states_during_a_slow_report(Restart::Always(BACKOFF));
    assert_eq!(states, [TaskState::Backoff { last: report.end, early_ends: 1, pause: 100 * MS }]);
}

#[test]
fn many_tasks_that_fail_at_the_same_time_keep_their_states_and_their_reports() {
    let runtime = runtime_with_workers(4);
    let (tasks, reports) = tasks_with_reports(&runtime);
    let (handles, alive) = start_tasks_that_fail_first(&tasks);

    for _ in 0..TASKS {
        alive.recv_timeout(LIMIT).expect("each task gets to its good run");
        let states = tasks.snapshot();
        assert_eq!(states.len(), TASKS, "a copy during the faults has each task");
        assert!(states.iter().all(|status| !matches!(status.state, TaskState::Ended(_))));
    }
    let good = |status: &TaskStatus| (&status.state, status.runs) == (&TaskState::Running, FAULTS + 1);
    assert!(tasks.snapshot().iter().all(good));
    assert_eq!(reports.try_iter().count(), TASKS * FAULTS as usize, "one report for each fault");
    assert_eq!(tasks.lost_reports(), 0);

    drop(runtime);
    let states = tasks.snapshot();
    assert_eq!(states.len(), handles.len());
    let stopped = |status: &TaskStatus| status.state == TaskState::Ended(End::Stopped);
    assert!(states.iter().all(stopped), "a runtime that stops gives each task its final state");
}

#[test]
fn a_reader_on_a_different_thread_never_sees_a_final_state_for_a_task_that_its_owner_stops() {
    let runtime = runtime_with_workers(4);
    let (tasks, _reports) = tasks_with_reports(&runtime);
    let (handles, alive) = start_tasks_that_fail_first(&tasks);
    for _ in 0..TASKS {
        alive.recv_timeout(LIMIT).expect("each task gets to its good run");
    }

    let all_dropped = AtomicBool::new(false);
    let ended_states_seen = std::thread::scope(|scope| {
        let reader = scope.spawn(|| {
            let mut ended = 0;
            while !all_dropped.load(Ordering::Relaxed) {
                ended += tasks.snapshot().iter().filter(|status| matches!(status.state, TaskState::Ended(_))).count();
            }
            ended
        });
        drop(handles);
        all_dropped.store(true, Ordering::Relaxed);
        reader.join().expect("the reader does not panic")
    });
    assert_eq!(ended_states_seen, 0);
    assert!(tasks.snapshot().is_empty());
}
