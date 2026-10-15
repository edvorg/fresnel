//! A child process in a job object: the Windows variant of [`Tree`].
//!
//! Windows has no process group. A job object holds the child and each process that it
//! starts, and one call ends them all. The child starts suspended, so it starts no process
//! before it is in the job; then its first thread runs. The system ends each process of the
//! job when the last handle of the job closes, also at a crash of the app.

use std::io;
use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle, RawHandle};
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use tokio::process::{Child, ChildStderr, ChildStdout, Command};
use tokio::time::timeout;
use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation, SetInformationJobObject,
    TerminateJobObject,
};
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, CREATE_SUSPENDED, OpenThread, ResumeThread, THREAD_SUSPEND_RESUME,
};

use crate::process::Program;

/// The wait for the end of a killed process: the end is certain, the limit is for a slow system.
const REAP_LIMIT: Duration = Duration::from_secs(1);
/// The exit code of each process that the stop of a tree ends.
const STOPPED: u32 = 1;

/// A started tree with the read ends of the output pipes of its child.
pub(crate) struct Started {
    pub(crate) tree: Tree,
    pub(crate) stdout: ChildStdout,
    pub(crate) stderr: ChildStderr,
}

/// Starts the program in a new job, with no console window: the app is not a console program,
/// and a plugin such as `doctl.exe` would get a window of its own.
pub(crate) fn start(program: &Program) -> io::Result<Started> {
    let job = Job::new()?;
    let mut command = Command::new(&program.path);
    command.args(&program.args).env_clear().envs(program.env.iter());
    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    command.creation_flags(CREATE_SUSPENDED | CREATE_NO_WINDOW);
    let mut child = command.spawn()?;
    let pipes = (child.stdout.take(), child.stderr.take());
    let started = match pipes {
        (Some(stdout), Some(stderr)) => job.assign(&child).and_then(|()| resume(&child)).map(|()| (stdout, stderr)),
        _ => Err(io::Error::other("the child has no output pipe")),
    };
    match started {
        Ok((stdout, stderr)) => Ok(Started { tree: Tree { child, job }, stdout, stderr }),
        Err(error) => {
            // A child that is not in the job, or that does not run, ends here. A suspended
            // child started nothing, and the kill reaches it in each state.
            match child.start_kill() {
                Ok(()) | Err(_) => {}
            }
            Err(error)
        }
    }
}

/// A child and its job. The drop closes the job, and the system ends each of its processes.
pub(crate) struct Tree {
    child: Child,
    job: Job,
}

impl Tree {
    /// Waits for the end of the child.
    pub(crate) async fn wait(&mut self) -> io::Result<ExitStatus> {
        self.child.wait().await
    }

    /// The child exited. A process that it started stays, as on unix: the close of the job
    /// then ends nothing. If the change fails, the close ends that process.
    pub(crate) fn forget(&mut self) -> bool {
        match self.job.kill_on_close(false) {
            Ok(()) | Err(_) => {}
        }
        true
    }

    /// Ends each process of the job and waits for the end of the child. True when the child
    /// is gone. A process of the job starts each child in the job, so none can escape it.
    pub(crate) async fn stop(&mut self) -> bool {
        self.job.terminate();
        timeout(REAP_LIMIT, self.child.wait()).await.is_ok()
    }
}

/// A job object of the system, owned by this value.
struct Job(OwnedHandle);

impl Job {
    /// A new job with no name, which ends its processes when its last handle closes.
    fn new() -> io::Result<Self> {
        // SAFETY: no security attributes and no name: the call makes a new job, and only the
        // handle that it returns reaches it.
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the handle is new and valid, and this value is its one owner.
        let job = Self(unsafe { OwnedHandle::from_raw_handle(handle) });
        job.kill_on_close(true)?;
        Ok(job)
    }

    fn handle(&self) -> RawHandle {
        self.0.as_raw_handle()
    }

    /// Sets or clears the limit that ends each process of the job at the close of the job.
    fn kill_on_close(&self, kill: bool) -> io::Result<()> {
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = if kill { JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE } else { 0 };
        let size = u32::try_from(size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>()).map_err(io::Error::other)?;
        let information = (&raw const limits).cast();
        // SAFETY: the handle is a job, and the pointer and the size describe `limits`, which
        // lives to the end of the call.
        let done =
            unsafe { SetInformationJobObject(self.handle(), JobObjectExtendedLimitInformation, information, size) };
        if done == 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
    }

    fn assign(&self, child: &Child) -> io::Result<()> {
        let process = child.raw_handle().ok_or_else(|| io::Error::other("the child ended before its job"))?;
        // SAFETY: both handles are valid: the job is owned here, and the child is not waited for.
        let done = unsafe { AssignProcessToJobObject(self.handle(), process) };
        if done == 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
    }

    /// Ends each process of the job. An error is a job with no process, or one that the
    /// system ends already: [`Tree::stop`] checks the end of the child.
    fn terminate(&self) {
        // SAFETY: the handle is a job that this value owns.
        unsafe { TerminateJobObject(self.handle(), STOPPED) };
    }
}

/// Lets the suspended child run: it has one thread, which the list of the threads of the
/// system names by the id of the child.
fn resume(child: &Child) -> io::Result<()> {
    let id = child.id().ok_or_else(|| io::Error::other("the child ended before it ran"))?;
    // SAFETY: a list of each thread of the system; the call has no pointer argument.
    let list = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if list == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the handle is new and valid, and this value is its one owner.
    let list = unsafe { OwnedHandle::from_raw_handle(list) };
    let size = u32::try_from(size_of::<THREADENTRY32>()).map_err(io::Error::other)?;
    let mut entry = THREADENTRY32 { dwSize: size, ..THREADENTRY32::default() };
    let mut resumed = false;
    // SAFETY: the list is valid, and `entry` is a record with its size set.
    let mut more = unsafe { Thread32First(list.as_raw_handle(), &raw mut entry) } != 0;
    while more {
        if entry.th32OwnerProcessID == id {
            resume_thread(entry.th32ThreadID)?;
            resumed = true;
        }
        // SAFETY: as for the first record.
        more = unsafe { Thread32Next(list.as_raw_handle(), &raw mut entry) } != 0;
    }
    if resumed { Ok(()) } else { Err(io::Error::other("the new process has no thread")) }
}

fn resume_thread(id: u32) -> io::Result<()> {
    // SAFETY: opens a thread by its id, with only the right to resume it, and no inheritance.
    let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, id) };
    if thread.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the handle is new and valid, and this value is its one owner.
    let thread = unsafe { OwnedHandle::from_raw_handle(thread) };
    // SAFETY: the handle has the right to resume the thread.
    if unsafe { ResumeThread(thread.as_raw_handle()) } == u32::MAX {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
