//! A child process and its process group: the unix variant of [`Tree`].
//!
//! The child runs in its own session. A new session has no controlling terminal, so an
//! interactive shell cannot stop on terminal job control. The child also leads a new process
//! group, so one signal reaches it and each process that it started.

use std::io;
use std::process::ExitStatus;
use std::time::Duration;

use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;
use tokio::net::unix::pipe::Receiver;
use tokio::time::{sleep, timeout};

use super::child::{self, Child};
use crate::process::Program;

/// The wait for the end of a killed process. `SIGKILL` cannot be refused, so the limit is
/// for a kernel that is slow.
const REAP_LIMIT: Duration = Duration::from_secs(1);
/// The most kills for the processes that stay in a group after its first process, and
/// the pause between two of them. A process that got the signal needs a moment to end.
const REST_KILLS: u32 = 100;
const REST_KILL_PAUSE: Duration = Duration::from_millis(2);

/// A started tree with the read ends of the output pipes of its child.
pub(crate) struct Started {
    pub(crate) tree: Tree,
    pub(crate) stdout: Receiver,
    pub(crate) stderr: Receiver,
}

/// Starts the program in a new session, as the leader of a new process group.
pub(crate) fn start(program: &Program) -> io::Result<Started> {
    let child::Started { child, stdout, stderr } = child::start(program)?;
    let id = Some(child.id());
    Ok(Started { tree: Tree { child, id }, stdout, stderr })
}

/// A child and its process group. One type owns the two, so the kill always comes before
/// the drop of the child, while the group id still belongs to this child.
pub(crate) struct Tree {
    child: Child,
    id: Option<Pid>,
}

impl Tree {
    /// Waits for the end of the child and collects it. A drop of the future loses no end.
    pub(crate) async fn wait(&mut self) -> io::Result<ExitStatus> {
        self.child.wait().await
    }

    /// The child exited and `wait` collected it. Its id can now belong to a new process, so
    /// no kill follows.
    pub(crate) fn forget(&mut self) -> bool {
        self.id = None;
        true
    }

    /// Sends `SIGKILL` to the group. The result of the call tells nothing useful: `ESRCH` is
    /// a group with no process, and macOS gives `EPERM` for a group of ended processes.
    /// [`Tree::stop`] checks the real result.
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
    pub(crate) async fn stop(&mut self) -> bool {
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

/// [`kill_the_rest`] on a thread of the blocking pool, after the collect of a child that
/// dropped before its end (`child::collect_later`).
pub(super) fn kill_the_rest_blocking(group: Pid) {
    for _ in 0..REST_KILLS {
        if killpg(group, Signal::SIGKILL).is_err() {
            return;
        }
        std::thread::sleep(REST_KILL_PAUSE);
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        self.kill();
    }
}

#[cfg(test)]
mod tests;
