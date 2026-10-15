//! The stop of a process group, through the run of the process module.

use std::panic::AssertUnwindSafe;

use futures_util::FutureExt as _;
use tokio::task::JoinSet;

use super::*;
use crate::process::{End, RunLimits, run};

const THREADS: usize = 8;
const ROUNDS: usize = 40;
/// The life of a long child. A start that waits for a long child needs this time.
const LONG: Duration = Duration::from_secs(2);
/// The tests that start hundreds of children take turns: the time of a short child has no
/// start of an other test in it (at the same time, one needed 1.05 s on the macOS runner).
static MANY_CHILDREN: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn turn() -> std::sync::MutexGuard<'static, ()> {
    MANY_CHILDREN.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn program(script: &str) -> Program {
    let env = [("PATH", "/usr/bin:/bin")].into_iter().collect();
    Program { path: "/bin/sh".into(), args: vec!["-c".into(), script.into()], env }
}

/// Starts a short child and gives the time to its end.
async fn short_run() -> Duration {
    let started = std::time::Instant::now();
    let limits = RunLimits { time: LONG, output_bytes: 1024 };
    let output = run(&program("true"), limits, |_| false).await.expect("the child starts");
    assert!(matches!(output.end, End::Exited(status) if status.success()), "{:?}", output.end);
    started.elapsed()
}

/// Some threads start a short child and a long one at the same moment, again and again.
/// A long child that held the pipe of a start on some other thread would make that start
/// wait for the end of the long child (milestone M0: 600 s). No child gets a descriptor
/// of the app but its own three (module `child`).
#[test]
fn a_spawn_does_not_wait_for_a_child_of_some_other_thread() {
    let _turn = turn();
    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(THREADS).enable_all().build();
    let slowest = runtime.expect("a runtime").block_on(async {
        let limits = RunLimits { time: LONG, output_bytes: 1024 };
        let (mut long_runs, mut slowest) = (JoinSet::new(), Duration::ZERO);
        for _ in 0..ROUNDS {
            let mut short_runs = JoinSet::new();
            for _ in 0..THREADS {
                long_runs.spawn(async move { run(&program("sleep 2"), limits, |_| false).await });
                short_runs.spawn(short_run());
            }
            while let Some(time) = short_runs.join_next().await {
                slowest = slowest.max(time.expect("no panic"));
            }
        }
        // The drop of the long runs kills their children.
        slowest
    });
    assert!(slowest < LONG / 2, "a short child needed {slowest:?}");
}

/// The exit hook of this shell writes the id of its process into a file and stays.
const SHELL_WITH_EXIT_HOOK: &str = r#"trap 'sh -c "echo \$\$ > \"\$HOOK_FILE\"; exec sleep 30"' EXIT; echo ready"#;

/// One run of the shell with the hook. The run ends when the output is complete, and
/// that is the moment at which the shell starts the process of its hook.
async fn run_with_exit_hook(hook_file: std::path::PathBuf) {
    let mut shell = program(SHELL_WITH_EXIT_HOOK);
    shell.env.set("HOOK_FILE", hook_file);
    let limits = RunLimits { time: LONG, output_bytes: 1024 };
    let output = run(&shell, limits, |out| out.ends_with(b"ready\n")).await.expect("the child starts");
    assert!(matches!(output.end, End::Complete) && output.stopped, "{:?}", output.end);
}

/// One run of the shell with the hook that drops at the moment of its output, as the
/// stop of the core drops the run of a plugin.
async fn drop_with_exit_hook(hook_file: std::path::PathBuf) {
    let mut shell = program(SHELL_WITH_EXIT_HOOK);
    shell.env.set("HOOK_FILE", hook_file);
    let limits = RunLimits { time: LONG, output_bytes: 1024 };
    let ready = tokio::sync::Notify::new();
    let run = run(&shell, limits, |out| {
        if out.ends_with(b"ready\n") {
            ready.notify_one();
        }
        false
    });
    tokio::select! {
        output = run => panic!("the run ended before its drop: {:?}", output.map(|output| output.end)),
        () = ready.notified() => {}
    }
}

/// A process that a member of the group starts at the moment of the kill is not in the
/// group yet. One kill does not reach it, and it stays after the app is gone.
#[test]
fn the_stop_of_a_group_leaves_no_process_that_a_member_starts_at_that_moment() {
    no_hook_stays(run_with_exit_hook);
}

/// The same for the drop of a run: the kill of a drop repeats after the collect too.
#[test]
fn the_drop_of_a_run_leaves_no_process_that_a_member_starts_at_that_moment() {
    no_hook_stays(drop_with_exit_hook);
}

/// A panic in the task of a run drops the run. The group ends, and the child is collected
/// with no later child: no zombie keeps its id.
#[tokio::test]
async fn a_panic_during_a_run_ends_its_group_and_leaves_no_zombie() {
    let (shell, limits, mut ids) =
        (program("sleep 30 & echo $$ $!; wait"), RunLimits { time: LONG, output_bytes: 1024 }, Vec::<i32>::new());
    let run = run(&shell, limits, |out| {
        if out.ends_with(b"\n") {
            ids = String::from_utf8_lossy(out).split_whitespace().map(|id| id.parse().expect("an id")).collect();
            panic!("a panic in the task of the run");
        }
        false
    });
    assert!(AssertUnwindSafe(run).catch_unwind().await.is_err(), "the run ended with the panic");
    assert_eq!(ids.len(), 2, "the ids of the shell and of its child: {ids:?}");
    let gone = async {
        while ids.iter().any(|id| nix::sys::signal::kill(Pid::from_raw(*id), None).is_ok()) {
            sleep(Duration::from_millis(10)).await;
        }
    };
    timeout(Duration::from_secs(10), gone).await.expect("the shell and its child are gone and collected");
}

/// Runs the shell with the hook many times at once and asserts that no hook stayed.
fn no_hook_stays<F: Future<Output = ()> + Send + 'static>(one_run: fn(std::path::PathBuf) -> F) {
    let _turn = turn();
    let folder = std::env::temp_dir().join(format!("fresnel-net-test-hook-{:08x}", fastrand::u32(..)));
    std::fs::create_dir(&folder).expect("a new folder");
    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(THREADS).enable_all().build();
    runtime.expect("a runtime").block_on(async {
        let mut runs = JoinSet::new();
        for number in 0..THREADS * ROUNDS {
            runs.spawn(one_run(folder.join(number.to_string())));
        }
        while let Some(done) = runs.join_next().await {
            done.expect("no panic");
        }
        // A process that no kill reached writes its file a moment after its start.
        tokio::time::sleep(Duration::from_millis(200)).await;
    });
    let mut alive = 0;
    for file in std::fs::read_dir(&folder).expect("the folder") {
        let text = std::fs::read_to_string(file.expect("an entry").path()).expect("the file of a hook");
        // A kill can reach a hook after it made its file and before it wrote its id.
        // That process is gone. One that stayed had the time above for its write.
        if text.is_empty() {
            continue;
        }
        let hook = Pid::from_raw(text.trim().parse().expect("a process id"));
        // The test ends a process that stayed, so that a failure leaves nothing.
        alive += usize::from(nix::sys::signal::kill(hook, Signal::SIGKILL).is_ok());
    }
    std::fs::remove_dir_all(&folder).expect("the folder goes away");
    assert_eq!(alive, 0, "processes of an exit hook stayed after the stop of their group");
}
