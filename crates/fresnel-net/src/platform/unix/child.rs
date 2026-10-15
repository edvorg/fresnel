//! The start of a child process and the wait for its end, with no fork.
//!
//! The app has threads. A fork runs the fork handlers of the system libraries in the
//! child before its exec. On macOS, the handler of libnotify stops the child with SIGKILL
//! when some other thread makes its first call into libnotify at that moment, as a DNS
//! lookup or a read of the time zone does ("os_once_t is corrupt", "crashed on child side
//! of fork pre-exec"). A test login shell ended in this way two times. `posix_spawn` runs
//! no code of the app or of a library in the child, and its flag SETSID gives the child a
//! session of its own.

use std::ffi::{CString, OsStr};
use std::fs::File;
use std::io;
use std::os::fd::AsRawFd as _;
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::process::ExitStatusExt as _;
use std::process::ExitStatus;
use std::sync::{Mutex, PoisonError};

use nix::errno::Errno;
use nix::libc::c_int;
use nix::spawn::{PosixSpawnAttr, PosixSpawnFileActions, PosixSpawnFlags, posix_spawn};
use nix::sys::signal::{SigSet, Signal};
use nix::sys::wait::{WaitPidFlag, WaitStatus, waitpid};
use nix::unistd::Pid;
use tokio::net::unix::pipe::Receiver;
use tokio::signal::unix::{SignalKind, signal};

use crate::process::Program;

/// The flags of `<spawn.h>` that `nix` does not name: a new session and, on macOS, no
/// descriptor of the app in the child but the three of the file actions. On Linux, each
/// descriptor of the app closes at the exec: Rust and tokio open each one so.
#[cfg(target_os = "macos")]
const FLAGS: c_int = 0x0400 | nix::libc::POSIX_SPAWN_CLOEXEC_DEFAULT;
#[cfg(not(target_os = "macos"))]
const FLAGS: c_int = nix::libc::POSIX_SPAWN_SETSID as c_int;

/// A started child with the read ends of its output pipes.
pub(super) struct Started {
    pub(super) child: Child,
    pub(super) stdout: Receiver,
    pub(super) stderr: Receiver,
}

/// One child process. The owner collects it with [`Child::wait`]. A child that drops
/// before that is collected by [`collect_later`], so it does not stay a zombie.
pub(super) struct Child {
    pid: Pid,
    status: Option<ExitStatus>,
    /// The signal of the end of each child. It exists before the start, so no end is lost.
    ended: tokio::signal::unix::Signal,
}

/// Starts the program with stdin from `/dev/null` and a pipe for stdout and for stderr.
/// Each signal has its default action and none is blocked in the child: the app ignores
/// SIGPIPE, and a child must not.
pub(super) fn start(program: &Program) -> io::Result<Started> {
    collect_ended();
    let ended = signal(SignalKind::child())?;
    let (stdout, stdout_end) = io::pipe()?;
    let (stderr, stderr_end) = io::pipe()?;
    let null = File::open("/dev/null")?;
    let mut actions = PosixSpawnFileActions::init()?;
    actions.add_dup2(null.as_raw_fd(), 0)?;
    actions.add_dup2(stdout_end.as_raw_fd(), 1)?;
    actions.add_dup2(stderr_end.as_raw_fd(), 2)?;
    let mut attributes = PosixSpawnAttr::init()?;
    attributes.set_sigmask(&SigSet::empty())?;
    attributes.set_sigdefault(&SigSet::from(Signal::SIGPIPE))?;
    let flags = PosixSpawnFlags::POSIX_SPAWN_SETSIGMASK | PosixSpawnFlags::POSIX_SPAWN_SETSIGDEF;
    attributes.set_flags(flags | PosixSpawnFlags::from_bits_retain(FLAGS))?;
    let path = c_string(program.path.as_os_str())?;
    let args = std::iter::once(Ok(path.clone())).chain(program.args.iter().map(|arg| c_string(arg)));
    let args = args.collect::<io::Result<Vec<CString>>>()?;
    let env = program.env.iter().map(|(name, value)| {
        let mut pair = name.to_owned();
        pair.push("=");
        pair.push(value);
        c_string(&pair)
    });
    let env = env.collect::<io::Result<Vec<CString>>>()?;
    // The read ends exist before the child: no error after the start leaves a child alone.
    let (stdout, stderr) = (Receiver::from_owned_fd(stdout.into())?, Receiver::from_owned_fd(stderr.into())?);
    let pid = posix_spawn(path.as_c_str(), &actions, &attributes, &args, &env)?;
    Ok(Started { child: Child { pid, status: None, ended }, stdout, stderr })
}

fn c_string(text: &OsStr) -> io::Result<CString> {
    CString::new(text.as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "a NUL byte in an argument or a variable"))
}

impl Child {
    pub(super) fn id(&self) -> Pid {
        self.pid
    }

    /// Waits for the end of the child and collects it. A drop of the future loses no end.
    pub(super) async fn wait(&mut self) -> io::Result<ExitStatus> {
        loop {
            if self.status.is_none() {
                self.status = look(self.pid)?;
            }
            if let Some(status) = self.status {
                return Ok(status);
            }
            if self.ended.recv().await.is_none() {
                return Err(io::Error::other("the signal driver of the runtime stopped"));
            }
            collect_ended();
        }
    }
}

impl Drop for Child {
    fn drop(&mut self) {
        if self.status.is_none() {
            collect_later(self.pid);
        }
    }
}

/// Collects a child that dropped before its end was collected, and then kills the rest of
/// its group, as [`super::tree::Tree::stop`] does.
///
/// The owner kills the process group before the drop of its child (`Tree`), so the end
/// comes a moment later. No later child may come to collect it: the stop of the core kills
/// the last plugin. Thus a thread of the blocking pool of the runtime waits for the end. It
/// is not a task of `Tasks`: a drop has none, the wait has no state to show, and SIGKILL
/// cannot be refused. The child leads its group (SETSID), so its id is the group id. The
/// one kill of the drop misses a process that a member starts at that moment, so the kill
/// repeats after the collect. With no runtime, the list [`UNCOLLECTED`] takes the child,
/// and no kill repeats.
fn collect_later(pid: Pid) {
    let mut dropped = Dropped(Some(pid));
    if let Ok(runtime) = tokio::runtime::Handle::try_current() {
        let wait = move || {
            if let Some(pid) = dropped.0.take() {
                wait_blocking(pid);
                super::tree::kill_the_rest_blocking(pid);
            }
        };
        // The handle drops: nothing reads the end of a child that its owner gave up.
        drop(runtime.spawn_blocking(wait));
    }
}

/// A child that dropped before its end was collected. One place collects it: the wait on a
/// thread of the blocking pool or else the list. Two could collect a process id that the
/// system gave to a new child after the first collect.
struct Dropped(Option<Pid>);

impl Drop for Dropped {
    /// With no runtime, or at the shutdown of the runtime, the wait does not run.
    fn drop(&mut self) {
        if let Some(pid) = self.0.take() {
            UNCOLLECTED.lock().unwrap_or_else(PoisonError::into_inner).push(pid);
        }
    }
}

/// Waits in the system for the end of a child and collects it. With no flags, `waitpid`
/// returns only for an end, an interruption, or an error such as no child with this id.
fn wait_blocking(pid: Pid) {
    while waitpid(pid, None) == Err(Errno::EINTR) {}
}

/// The children that dropped before their end was collected, with no runtime for the wait.
/// A later start or end collects each one that ended.
static UNCOLLECTED: Mutex<Vec<Pid>> = Mutex::new(Vec::new());

/// Collects each child of [`UNCOLLECTED`] that ended. A wait with `WNOHANG` does not block.
fn collect_ended() {
    let mut pids = UNCOLLECTED.lock().unwrap_or_else(PoisonError::into_inner);
    pids.retain(|pid| matches!(look(*pid), Ok(None)));
}

/// The end of a child, if it ended, and then the child is collected. `None` while it runs.
fn look(pid: Pid) -> io::Result<Option<ExitStatus>> {
    loop {
        match waitpid(pid, Some(WaitPidFlag::WNOHANG)) {
            // The form of a status of `wait`: the exit code above the low 8 bits, or the
            // signal in the low 7 bits with the core dump flag.
            Ok(WaitStatus::Exited(_, code)) => return Ok(Some(ExitStatus::from_raw(code << 8))),
            Ok(WaitStatus::Signaled(_, signal, core)) => {
                return Ok(Some(ExitStatus::from_raw(signal as c_int | if core { 0x80 } else { 0 })));
            }
            Ok(_) => return Ok(None),
            Err(Errno::EINTR) => {}
            Err(error) => return Err(error.into()),
        }
    }
}

#[cfg(test)]
mod tests;
