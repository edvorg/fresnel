//! One run of a child process: the login shell or the credential plugin.
//!
//! The child and each process that it starts form one tree (`platform`): a process group
//! in a new session on unix, a job object on Windows. Each run has a time limit and an
//! output limit, and each end but a normal exit kills the tree.

use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::process::ExitStatus;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::time::{Instant, timeout, timeout_at};

use crate::env::ProcessEnv;
use crate::platform::{self, Started, Tree};

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

/// A program to start: its file, its arguments and its whole environment. No `Debug`: the
/// environment can hold a secret.
pub(crate) struct Program {
    pub(crate) path: PathBuf,
    pub(crate) args: Vec<OsString>,
    pub(crate) env: ProcessEnv,
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
/// A drop of the future kills the tree.
pub(crate) async fn run(
    program: &Program,
    limits: RunLimits,
    complete: impl FnMut(&[u8]) -> bool,
) -> io::Result<Output> {
    let Started { mut tree, stdout, stderr } = platform::start(program)?;
    let (mut out, mut err) = (Vec::new(), Vec::new());
    // The buffers live outside the timed future, so they keep their bytes after a timeout.
    let reads = Reads { stdout, stderr, out: &mut out, err: &mut err, cap: limits.output_bytes };
    let deadline = Instant::now() + limits.time;
    let end = timeout_at(deadline, reads.until_end(&mut tree, complete)).await.unwrap_or(End::TimedOut);
    let stopped = match end {
        End::Exited(_) => tree.forget(),
        End::Complete | End::TimedOut | End::TooMuchOutput | End::Failed(_) => tree.stop().await,
    };
    Ok(Output { stdout: out, stderr: err, end, stopped })
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
    async fn until_end(mut self, child: &mut Tree, mut complete: impl FnMut(&[u8]) -> bool) -> End {
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
