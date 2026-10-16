//! The end of a session: the close that its owner asks for, the second exec that ends the
//! processes of a shell, the delete of the pod of a node shell, and the final state.
//!
//! A WebSocket close and a reset leave a shell in the pod (terminal spike, X11). Ctrl-D ends
//! a shell at its prompt; a hangup through a second exec ends it and its jobs in each other
//! case. The status of the process decides the final state, also after a close.

use std::time::Duration;

use tokio::time::timeout;

use super::pump::{self, Streams};
use super::script::{self, Mark};
use super::{Parts, Wake};
use crate::cluster::Fate;
use crate::error::CoreError;
use crate::ops::{Target, shell};
use crate::store::ObjectKey;
use crate::stream::{Exec, ExecEnd, ExecSession, Exit, Output};
use crate::terminal::{SessionState, Terminal};

/// The byte of Ctrl-D: the end of the input at the prompt of a shell.
const CTRL_D: u8 = 0x04;
/// The wait for the end of a shell after Ctrl-D, as the spike proposed: a shell at its prompt
/// ends in some milliseconds.
const CTRL_D_WAIT: Duration = Duration::from_secs(1);
/// The longest wait for the end of a stream after its close. The socket ends within its
/// close limit; this one only holds a socket that never ends.
const CLOSE_WAIT: Duration = Duration::from_secs(30);
/// The longest second exec, with its connect: a busy node takes some seconds.
const SIGNAL_LIMIT: Duration = Duration::from_secs(20);

/// Closes an open stream: Ctrl-D first to a shell of the app, then the close of the socket.
/// An attach only closes: the main process of its container is not one of the app.
pub(super) async fn close(
    stream: &mut ExecSession,
    terminal: &Terminal,
    (streams, wake): (Streams, &Wake),
    shell: bool,
) {
    terminal.set_state(SessionState::Closing);
    wake();
    terminal.drop_input();
    if shell {
        // The queue of the socket can be full of input that the program does not read; the
        // Ctrl-D then waits behind it, and the signal ends the shell.
        let sent = timeout(CTRL_D_WAIT, stream.input().send(&[CTRL_D])).await;
        if matches!(sent, Ok(Ok(()))) {
            pump::drain(stream, terminal, (streams.tty, wake), CTRL_D_WAIT).await;
        }
    }
    stream.close();
    pump::drain(stream, terminal, (streams.tty, wake), CLOSE_WAIT).await;
}

/// Sends a hangup to each process of the session through a second exec into its container.
/// Gives the count of processes that got it.
pub(super) async fn signal(parts: &Parts, pod: &ObjectKey, container: &str, mark: &Mark) -> Result<u32, CoreError> {
    let exec = Exec {
        pod: pod.clone(),
        container: Some(container.to_owned()),
        command: Some(script::end(mark)),
        tty: false,
        stdin: false,
    };
    let run = async {
        let mut session = ExecSession::open(&parts.gateway, &exec, parts.journal.clone()).await?;
        let mut printed = Vec::new();
        while let Some(output) = session.next().await {
            if let Output::Stdout(data) = output {
                printed.extend_from_slice(&data);
            }
        }
        match session.end() {
            Some(ExecEnd::Exited(Exit::Code(0))) => {
                let text = String::from_utf8_lossy(&printed);
                text.trim().parse().map_err(|_| CoreError::invalid("the second exec printed no count"))
            }
            Some(ExecEnd::Exited(exit)) => Err(CoreError::invalid(exit.to_string())),
            Some(ExecEnd::Socket(end)) => Err(CoreError::invalid(format!("its stream ended: {end}"))),
            Some(ExecEnd::Invalid(text)) => Err(CoreError::invalid(text)),
            None => Err(CoreError::invalid("its stream ended with no state")),
        }
    };
    let limit = SIGNAL_LIMIT.as_secs();
    timeout(SIGNAL_LIMIT, run).await.unwrap_or_else(|_| Err(CoreError::invalid(format!("no end in {limit} s"))))
}

/// Deletes the pod of a node shell. A pod that is gone counts as deleted.
pub(super) async fn delete_pod(parts: &Parts, pod: &Target) -> Result<(), CoreError> {
    match parts.gateway.send::<serde_json::Value>(shell::delete_pod_now(pod)?).await {
        Ok(_) => Ok(()),
        Err(error) if error.error.status() == Some(404) => Ok(()),
        Err(error) if error.fate == Fate::Unknown => {
            let read =
                parts.gateway.get::<serde_json::Value>(shell::read_pod(&pod.key.namespace, &pod.key.name)?).await;
            match read {
                Err(read) if read.status() == Some(404) => Ok(()),
                _ => Err(error.error),
            }
        }
        Err(error) => Err(error.error),
    }
}

/// The final state of a stream by its end: the exit code decides, also after a close.
pub(super) fn final_state(end: Option<ExecEnd>, closed: bool) -> SessionState {
    match end {
        Some(ExecEnd::Exited(Exit::Code(code))) => SessionState::Exited { code },
        Some(ExecEnd::Exited(error @ Exit::Error { .. })) => SessionState::Failed { reason: error.to_string() },
        Some(ExecEnd::Invalid(reason)) => SessionState::Failed { reason },
        Some(ExecEnd::Socket(_)) if closed => SessionState::Ended { reason: "the app closed the session".to_owned() },
        Some(ExecEnd::Socket(end)) if end.is_clean() => SessionState::Ended { reason: end.to_string() },
        Some(ExecEnd::Socket(end)) => SessionState::Failed { reason: end.to_string() },
        None => SessionState::Failed { reason: "the stream did not end in time".to_owned() },
    }
}

#[cfg(test)]
mod tests {
    use fresnel_net::{SocketEnd, SocketFailure};

    use super::*;

    #[test]
    fn the_exit_code_decides_the_final_state_and_a_close_ends_with_no_fault() {
        let state = |end, closed| final_state(Some(end), closed);
        assert_eq!(state(ExecEnd::Exited(Exit::Code(130)), true), SessionState::Exited { code: 130 });
        let missing = Exit::Error { reason: "InternalError".to_owned(), message: "no sh".to_owned() };
        let reason = "the server ended the process: InternalError: no sh".to_owned();
        assert_eq!(state(ExecEnd::Exited(missing), false), SessionState::Failed { reason });
        let silent = SocketEnd::Failed(SocketFailure::Silent { quiet: Duration::from_secs(15) });
        let reason = "silent: no frame for 15.0 s, also no answer to a ping".to_owned();
        assert_eq!(state(ExecEnd::Socket(silent.clone()), false), SessionState::Failed { reason });
        let closed = SessionState::Ended { reason: "the app closed the session".to_owned() };
        assert_eq!(state(ExecEnd::Socket(silent), true), closed);
        let ended = SessionState::Ended { reason: "ended by the server".to_owned() };
        assert_eq!(state(ExecEnd::Socket(SocketEnd::EndedByServer), false), ended);
        assert!(matches!(final_state(None, false), SessionState::Failed { .. }));
    }
}
