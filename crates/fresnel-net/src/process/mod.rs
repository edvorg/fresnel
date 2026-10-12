//! One run of a child process: the login shell or the credential plugin.
//!
//! The child runs in its own session. A new session has no controlling terminal, so an
//! interactive shell cannot stop on terminal job control. The child also leads a new process
//! group, so one signal reaches it and each process that it started. Each run has a time
//! limit and an output limit, and each end but a normal exit kills the group.

use std::io;
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use nix::sys::signal::{Signal, killpg};
use nix::unistd::{Pid, setsid};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::{Child, Command};
use tokio::time::{Instant, timeout, timeout_at};

/// The wait for the end of a killed process. `SIGKILL` cannot be refused, so the limit is
/// for a kernel that is slow.
const REAP_LIMIT: Duration = Duration::from_secs(1);
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

/// Runs the command to an end. `complete` reads the stdout bytes so far; with `true` the run
/// ends at once. An error is a child that did not start.
///
/// A drop of the future kills the process group.
pub(crate) async fn run(
    mut command: Command,
    limits: RunLimits,
    complete: impl FnMut(&[u8]) -> bool,
) -> io::Result<Output> {
    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    // SAFETY: the closure runs between fork and exec. `setsid` is async-signal-safe, and the
    // closure does not allocate and takes no lock.
    unsafe { command.pre_exec(|| setsid().map(drop).map_err(io::Error::from)) };
    let mut group = Group::of(command.spawn()?);
    let (Some(stdout), Some(stderr)) = (group.child.stdout.take(), group.child.stderr.take()) else {
        return Err(io::Error::other("the child has no output pipe"));
    };
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
        let id = child.id().and_then(|id| i32::try_from(id).ok()).map(Pid::from_raw);
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
    async fn stop(&mut self) -> bool {
        self.kill();
        timeout(REAP_LIMIT, self.child.wait()).await.is_ok()
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
