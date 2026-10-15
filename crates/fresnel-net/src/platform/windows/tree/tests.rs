//! The job of a child: the stop and the drop end each process that the child started, and a
//! normal exit leaves them, as on unix.

use std::os::windows::io::{FromRawHandle as _, OwnedHandle};

use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, TerminateProcess, WaitForSingleObject,
};

use super::*;
use crate::env::ProcessEnv;
use crate::process::{End, RunLimits, run};

const LONG: Duration = Duration::from_secs(30);

/// `cmd.exe /c <line>` with the environment of the test process: `ping.exe` needs the
/// variables of the system.
fn cmd(line: &str) -> Program {
    let env = ProcessEnv::from_process();
    let path = env.find_program("cmd", None).expect("cmd.exe in PATH");
    Program { path, args: vec!["/c".into(), line.into()], env }
}

/// A process that runs 60 seconds and writes nothing.
const PING: &str = "ping -n 60 127.0.0.1 > nul";

/// The ids of the processes whose parent is `parent`.
fn children_of(parent: u32) -> Vec<u32> {
    // SAFETY: a list of each process of the system; the call has no pointer argument.
    let list = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    assert_ne!(list, INVALID_HANDLE_VALUE, "a list of the processes");
    // SAFETY: the handle is new and valid, and this value is its one owner.
    let list = unsafe { OwnedHandle::from_raw_handle(list) };
    let size = u32::try_from(size_of::<PROCESSENTRY32W>()).expect("a small record");
    let mut entry = PROCESSENTRY32W { dwSize: size, ..PROCESSENTRY32W::default() };
    let mut children = Vec::new();
    // SAFETY: the list is valid, and `entry` is a record with its size set.
    let mut more = unsafe { Process32FirstW(list.as_raw_handle(), &raw mut entry) } != 0;
    while more {
        if entry.th32ParentProcessID == parent {
            children.push(entry.th32ProcessID);
        }
        // SAFETY: as for the first record.
        more = unsafe { Process32NextW(list.as_raw_handle(), &raw mut entry) } != 0;
    }
    children
}

/// Waits until `parent` has a child, and gives its id.
async fn first_child_of(parent: u32) -> u32 {
    let wait = async {
        loop {
            if let Some(child) = children_of(parent).first() {
                return *child;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    timeout(LONG, wait).await.expect("the child of cmd.exe starts")
}

/// True when the process ends in `limit`. A process that is gone is ended.
fn ends_in(process: u32, limit: Duration) -> bool {
    // SAFETY: opens a process by its id with only the right to wait for it.
    let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, process) };
    if handle.is_null() {
        return true;
    }
    // SAFETY: the handle is new and valid, and this value is its one owner.
    let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
    let millis = u32::try_from(limit.as_millis()).expect("a short limit");
    // SAFETY: the handle has the right to wait.
    unsafe { WaitForSingleObject(handle.as_raw_handle(), millis) == WAIT_OBJECT_0 }
}

/// Ends a process that a test left.
fn end(process: u32) {
    // SAFETY: opens a process by its id with only the right to end it.
    let handle = unsafe { OpenProcess(PROCESS_TERMINATE, 0, process) };
    if !handle.is_null() {
        // SAFETY: the handle is new and valid, and this value is its one owner.
        let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
        // SAFETY: the handle has the right to end the process.
        unsafe { TerminateProcess(handle.as_raw_handle(), 1) };
    }
}

#[tokio::test]
async fn a_run_gives_the_output_and_the_exit_code_of_the_child() {
    let limits = RunLimits { time: LONG, output_bytes: 1024 };
    let output = run(&cmd("echo fresnel & exit 7"), limits, |_| false).await.expect("the child starts");
    assert!(matches!(output.end, End::Exited(status) if status.code() == Some(7)), "{:?}", output.end);
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "fresnel");
}

#[tokio::test]
async fn the_stop_of_a_tree_ends_the_process_that_its_child_started() {
    let Started { mut tree, .. } = start(&cmd(PING)).expect("a start");
    let ping = first_child_of(tree.child.id().expect("a running child")).await;
    assert!(tree.stop().await, "cmd.exe is gone");
    assert!(ends_in(ping, Duration::from_secs(10)), "ping.exe ends with its job");
}

#[tokio::test]
async fn the_drop_of_a_tree_ends_the_process_that_its_child_started() {
    let Started { tree, .. } = start(&cmd(PING)).expect("a start");
    let (shell, ping) = {
        let shell = tree.child.id().expect("a running child");
        (shell, first_child_of(shell).await)
    };
    drop(tree);
    assert!(ends_in(shell, Duration::from_secs(10)) && ends_in(ping, Duration::from_secs(10)), "the job ends both");
}

#[tokio::test]
async fn a_time_limit_ends_the_tree() {
    let limits = RunLimits { time: Duration::from_millis(500), output_bytes: 1024 };
    let output = run(&cmd(PING), limits, |_| false).await.expect("the child starts");
    assert!(matches!(output.end, End::TimedOut) && output.stopped, "{:?}", output.end);
}

/// After a normal exit, a process that the child left keeps running, as on unix.
#[tokio::test]
async fn a_normal_exit_leaves_the_process_that_the_child_started() {
    let Started { mut tree, .. } = start(&cmd(&format!("start /b {PING}"))).expect("a start");
    let ping = first_child_of(tree.child.id().expect("a running child")).await;
    assert!(tree.wait().await.expect("the end of cmd.exe").success());
    tree.forget();
    drop(tree);
    let stayed = !ends_in(ping, Duration::from_millis(500));
    end(ping);
    assert!(stayed, "ping.exe stays after the normal exit of cmd.exe");
}
