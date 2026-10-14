//! One run of a child process: the login shell or the credential plugin.
//!
//! The child runs in its own session. A new session has no controlling terminal, so an
//! interactive shell cannot stop on terminal job control. The child also leads a new process
//! group, so one signal reaches it and each process that it started. Each run has a time
//! limit and an output limit, and each end but a normal exit kills the group.
//!
//! - `child`: the start with no fork, and the wait for the end.

mod child;

use std::io;
use std::process::ExitStatus;
use std::time::Duration;

use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::time::{Instant, sleep, timeout, timeout_at};

pub(crate) use self::child::Program;
use self::child::{Child, Started};

/// The wait for the end of a killed process. `SIGKILL` cannot be refused, so the limit is
/// for a kernel that is slow.
const REAP_LIMIT: Duration = Duration::from_secs(1);
/// The most kills for the processes that stay in a group after its first process, and
/// the pause between two of them. A process that got the signal needs a moment to end.
const REST_KILLS: u32 = 100;
const REST_KILL_PAUSE: Duration = Duration::from_millis(2);
/// After the exit of the child, its last bytes are in the pipe already. A helper process
/// that it left can hold the pipe open, so the read does not wait for the end of the pipe.
const DRAIN_GRACE: Duration = Duration::from_millis(100);

/// The limits of one run.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RunLimits {
    pub(crate) time: Duration,
    /// For stdout and for stderr, each.
    pub(crate) output_bytes: usize,
}

/// What one run gave.
pub(crate) struct Output {
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
    pub(crate) end: End,
    /// False when the child was not gone a moment after the kill.
    pub(crate) stopped: bool,
}

/// Why the run ended.
#[derive(Debug)]
pub(crate) enum End {
    Exited(ExitStatus),
    /// The caller has all the output that it needs. The run did not wait for the exit.
    Complete,
    TimedOut,
    TooMuchOutput,
    Failed(io::Error),
}

/// Runs the program to an end. `complete` reads the stdout bytes so far; with `true` the run
/// ends at once. An error is a child that did not start.
///
/// A drop of the future kills the process group.
pub(crate) async fn run(
    program: &Program,
    limits: RunLimits,
    complete: impl FnMut(&[u8]) -> bool,
) -> io::Result<Output> {
    let Started { child, stdout, stderr } = child::start(program)?;
    let mut group = Group::of(child);
    let (mut out, mut err) = (Vec::new(), Vec::new());
    // The buffers live outside the timed future, so they keep their bytes after a timeout.
    let reads = Reads { stdout, stderr, out: &mut out, err: &mut err, cap: limits.output_bytes };
    let deadline = Instant::now() + limits.time;
    let end = timeout_at(deadline, reads.until_end(&mut group.child, complete)).await.unwrap_or(End::TimedOut);
    let stopped = match end {
        End::Exited(_) => group.forget(),
        End::Complete | End::TimedOut | End::TooMuchOutput | End::Failed(_) => group.stop().await,
    };
    Ok(Output { stdout: out, stderr: err, end, stopped })
}

/// A child and its process group. One type owns the two, so the kill always comes before
/// the drop of the child, while the group id still belongs to this child.
struct Group {
    child: Child,
    id: Option<Pid>,
}

impl Group {
    fn of(child: Child) -> Self {
        let id = Some(child.id());
        Self { child, id }
    }

    /// The child exited and `wait` collected it. Its id can now belong to a new process, so
    /// no kill follows.
    fn forget(&mut self) -> bool {
        self.id = None;
        true
    }

    /// Sends `SIGKILL` to the group. The result of the call tells nothing useful: `ESRCH` is
    /// a group with no process, and macOS gives `EPERM` for a group of ended processes.
    /// [`Group::stop`] checks the real result.
    fn kill(&mut self) {
        if let Some(id) = self.id.take() {
            match killpg(id, Signal::SIGKILL) {
                Ok(()) | Err(_) => {}
            }
        }
    }

    /// Kills the group and collects the child. True when the child is gone.
    ///
    /// A process that a member of the group starts at the moment of a kill is not in the
    /// group yet, and that kill does not reach it. In a test, the command of the exit hook
    /// of a shell stayed for this reason in 2 of 12 runs. Thus the kill repeats until the
    /// group has no process.
    async fn stop(&mut self) -> bool {
        let id = self.id;
        self.kill();
        let gone = timeout(REAP_LIMIT, self.child.wait()).await.is_ok();
        if let Some(id) = id {
            kill_the_rest(id).await;
        }
        gone
    }
}

/// Kills the processes that are in the group after the collect of its first process, until
/// the system reports a group with no process.
///
/// The id of a group with a process cannot become the id of a new process. The id of an
/// empty group can, but only after the system gave each other id to a new process. The first
/// kill here follows the collect at once, so it reaches the old group or no process.
async fn kill_the_rest(group: Pid) {
    for _ in 0..REST_KILLS {
        if killpg(group, Signal::SIGKILL).is_err() {
            return;
        }
        sleep(REST_KILL_PAUSE).await;
    }
}

impl Drop for Group {
    fn drop(&mut self) {
        self.kill();
    }
}

/// The two pipes of a child and the buffers for their bytes.
struct Reads<'a, O, E> {
    stdout: O,
    stderr: E,
    out: &'a mut Vec<u8>,
    err: &'a mut Vec<u8>,
    cap: usize,
}

impl<O: AsyncRead + Unpin, E: AsyncRead + Unpin> Reads<'_, O, E> {
    /// Reads until the child exits, the caller has its output, or a limit is passed. It does
    /// not wait for the end of a pipe: a process that the child started can keep one open.
    async fn until_end(mut self, child: &mut Child, mut complete: impl FnMut(&[u8]) -> bool) -> End {
        let (mut out_open, mut err_open) = (true, true);
        loop {
            if complete(self.out) {
                return End::Complete;
            }
            if self.over_the_cap() {
                return End::TooMuchOutput;
            }
            let read = tokio::select! {
                read = self.stdout.read_buf(self.out), if out_open => read.map(|count| out_open = count > 0),
                read = self.stderr.read_buf(self.err), if err_open => read.map(|count| err_open = count > 0),
                status = child.wait() => return match status {
                    Ok(status) => self.after_exit(status, complete).await,
                    Err(error) => End::Failed(error),
                },
            };
            if let Err(error) = read {
                return End::Failed(error);
            }
        }
    }

    async fn after_exit(mut self, status: ExitStatus, mut complete: impl FnMut(&[u8]) -> bool) -> End {
        let drained = match drain(&mut self.stdout, self.out, self.cap).await {
            Ok(()) => drain(&mut self.stderr, self.err, self.cap).await,
            Err(error) => Err(error),
        };
        match drained {
            Err(error) => End::Failed(error),
            Ok(()) if complete(self.out) => End::Complete,
            Ok(()) if self.over_the_cap() => End::TooMuchOutput,
            Ok(()) => End::Exited(status),
        }
    }

    fn over_the_cap(&self) -> bool {
        self.out.len() > self.cap || self.err.len() > self.cap
    }
}

/// Reads the bytes that are in the pipe now.
async fn drain(pipe: &mut (impl AsyncRead + Unpin), into: &mut Vec<u8>, cap: usize) -> io::Result<()> {
    while into.len() <= cap {
        match timeout(DRAIN_GRACE, pipe.read_buf(into)).await {
            Ok(Ok(0)) | Err(_) => break,
            Ok(Ok(_)) => {}
            Ok(Err(error)) => return Err(error),
        }
    }
    Ok(())
}

/// The last characters of the stderr text of a child, for the UI.
pub(crate) fn tail(bytes: &[u8], characters: usize) -> String {
    let text = String::from_utf8_lossy(bytes);
    let skip = text.chars().count().saturating_sub(characters);
    text.chars().skip(skip).collect::<String>().trim().to_owned()
}

#[cfg(test)]
mod tests {
    use tokio::task::JoinSet;

    use super::*;

    const THREADS: usize = 8;
    const ROUNDS: usize = 40;
    /// The life of a long child. A start that waits for a long child needs this time.
    const LONG: Duration = Duration::from_secs(2);

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

    /// A process that a member of the group starts at the moment of the kill is not in the
    /// group yet. One kill does not reach it, and it stays after the app is gone.
    #[test]
    fn the_stop_of_a_group_leaves_no_process_that_a_member_starts_at_that_moment() {
        let folder = std::env::temp_dir().join(format!("fresnel-net-test-hook-{:08x}", fastrand::u32(..)));
        std::fs::create_dir(&folder).expect("a new folder");
        let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(THREADS).enable_all().build();
        runtime.expect("a runtime").block_on(async {
            let mut runs = JoinSet::new();
            for number in 0..THREADS * ROUNDS {
                runs.spawn(run_with_exit_hook(folder.join(number.to_string())));
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
}
