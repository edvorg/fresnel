//! The start with no fork: no fork handler runs in the child, the child leads a session of
//! its own, and SIGPIPE ends it as it ends a program of a terminal.

use std::sync::Once;

use nix::sys::signal::kill;
use nix::unistd::{getpgid, getsid};

use super::*;

/// The status of a child in which the fork handler of this test ran.
const IN_FORK_HANDLER: c_int = 42;

extern "C" fn end_the_child() {
    // SAFETY: `_exit` is async-signal-safe, so a fork handler can call it.
    unsafe { nix::libc::_exit(IN_FORK_HANDLER) }
}

fn program(script: &str) -> Program {
    let env = [("PATH", "/usr/bin:/bin")].into_iter().collect();
    Program { path: "/bin/sh".into(), args: vec!["-c".into(), script.into()], env }
}

/// A fork runs each fork handler of the process in the child, and on macOS the one of
/// libnotify can stop the child with SIGKILL. The handler of this test ends the child with
/// status 42, so a start with a fork fails the test.
#[tokio::test]
async fn the_child_runs_no_fork_handler_and_leads_a_session_of_its_own() {
    static HANDLER: Once = Once::new();
    HANDLER.call_once(|| {
        let handler: unsafe extern "C" fn() = end_the_child;
        // SAFETY: the handler only calls `_exit`, and it is registered one time.
        let result = unsafe { nix::libc::pthread_atfork(None, None, Some(handler)) };
        assert_eq!(result, 0, "the fork handler is registered");
    });

    let Started { mut child, .. } = start(&program("exit 7")).expect("a start");
    let status = child.wait().await.expect("an end");
    assert_eq!(status.code(), Some(7), "{status}: status {IN_FORK_HANDLER} is the fork handler");

    let Started { mut child, .. } = start(&program("exec sleep 30")).expect("a start");
    let pid = child.id();
    assert_eq!((getsid(Some(pid)), getpgid(Some(pid))), (Ok(pid), Ok(pid)), "it leads its session and its group");
    kill(pid, Signal::SIGKILL).expect("the child runs");
    assert_eq!(child.wait().await.expect("an end").signal(), Some(Signal::SIGKILL as c_int));
}

/// The app ignores SIGPIPE. A child gets the default action, so a writer to a pipe with no
/// reader ends, and does not write to it for all time.
#[tokio::test]
async fn a_child_whose_reader_is_gone_ends_with_sigpipe() {
    let Started { mut child, stdout, .. } = start(&program("exec yes")).expect("a start");
    drop(stdout);
    let status = child.wait().await.expect("an end");
    assert_eq!(status.signal(), Some(Signal::SIGPIPE as c_int), "{status}");
}

#[tokio::test]
async fn a_nul_byte_in_an_argument_is_an_error_and_starts_nothing() {
    let error = start(&program("echo a\0b")).err().expect("no start");
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{error}");
}
