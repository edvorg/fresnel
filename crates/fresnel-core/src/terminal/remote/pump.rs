//! The run of an open stream: the output of the remote program into the screen; the size,
//! the keys and a paste from the terminal to the program; the state of the socket on the
//! line of the tab.
//!
//! The reads and the writes run in one loop, and no arm waits in its body: a write that
//! waits for a program that reads no input stops no read of its output, and no close.

use std::borrow::Cow;
use std::time::Duration;

use fresnel_net::SocketState;
use tokio::sync::watch;
use tokio::time::sleep;

use super::Wake;
use crate::error::CoreError;
use crate::stream::{ExecSession, Input, Output};
use crate::terminal::{SessionState, Terminal};

/// The pause between two pieces of a paste. The API server lost the end of a shell after a
/// paste of 1 MiB in one frame, and kept each byte with pieces of 16 KiB each 5 ms (terminal
/// spike review, attack A2).
const PASTE_PAUSE: Duration = Duration::from_millis(5);

/// The streams of a session: a terminal, and the input of the program.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Streams {
    pub(super) tty: bool,
    pub(super) stdin: bool,
}

/// Why the pump ended.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Pumped {
    /// The stream ended: the session tells how.
    Ended,
    /// The owner asked for the close.
    Close,
}

/// Carries the bytes of an open stream until its end or a close of the owner.
pub(super) async fn pump(
    stream: &mut ExecSession,
    terminal: &Terminal,
    (streams, wake): (Streams, &Wake),
    close: &mut watch::Receiver<bool>,
) -> Pumped {
    let input = stream.input();
    let writer = write(terminal, &input, streams);
    tokio::pin!(writer);
    let mut writing = true;
    let mut states = stream.states();
    show(terminal, &states.borrow_and_update());
    wake();
    loop {
        if *close.borrow_and_update() {
            return Pumped::Close;
        }
        let deadline = terminal.sync_deadline().map(tokio::time::Instant::from_std);
        tokio::select! {
            output = stream.next() => match output {
                Some(Output::Stdout(data) | Output::Stderr(data)) => {
                    terminal.feed(&lines(&data, streams.tty));
                    wake();
                }
                None => return Pumped::Ended,
            },
            // A write fails after the end of the socket, whose read then ends too.
            _ = &mut writer, if writing => writing = false,
            () = sleep_until(deadline), if deadline.is_some() => {
                terminal.end_sync();
                wake();
            }
            Ok(()) = states.changed() => {
                show(terminal, &states.borrow_and_update());
                wake();
            }
            Ok(()) = close.changed() => {}
        }
    }
}

/// Feeds the output of a program to its end, for at most `limit`. True at the end.
pub(super) async fn drain(
    stream: &mut ExecSession,
    terminal: &Terminal,
    (tty, wake): (bool, &Wake),
    limit: Duration,
) -> bool {
    let deadline = tokio::time::Instant::now() + limit;
    loop {
        match tokio::time::timeout_at(deadline, stream.next()).await {
            Ok(Some(Output::Stdout(data) | Output::Stderr(data))) => {
                terminal.feed(&lines(&data, tty));
                wake();
            }
            Ok(None) => return true,
            Err(_) => return false,
        }
    }
}

/// Sends the size first, then each new size and the input in pieces, until a write fails.
async fn write(terminal: &Terminal, input: &Input, streams: Streams) -> Result<(), CoreError> {
    if streams.tty {
        let size = terminal.size();
        input.resize(size.columns, size.rows).await?;
    }
    loop {
        if let Some(size) = terminal.take_size().filter(|_| streams.tty) {
            input.resize(size.columns, size.rows).await?;
        }
        let mut first = true;
        while let Some(piece) = terminal.take_input() {
            // A program with no input takes no key: the key goes nowhere.
            if !streams.stdin {
                continue;
            }
            if !first {
                sleep(PASTE_PAUSE).await;
            }
            input.send(&piece).await?;
            first = false;
        }
        terminal.wanted().await;
    }
}

/// The state of the session for a state of its socket. The end of the socket ends the read,
/// and the session then tells the final state.
fn show(terminal: &Terminal, state: &SocketState) {
    let shown = match state {
        SocketState::Open => SessionState::Open,
        SocketState::WriteWaits { since } => SessionState::InputWaits { since: *since },
        SocketState::Closing => SessionState::Closing,
        SocketState::Ended(_) => return,
    };
    terminal.set_state(shown);
}

/// The output for the screen. With no terminal, a line ends with a line feed only, and the
/// screen needs a carriage return too.
fn lines(data: &[u8], tty: bool) -> Cow<'_, [u8]> {
    if tty || !data.contains(&b'\n') {
        return Cow::Borrowed(data);
    }
    let mut out = Vec::with_capacity(data.len() + data.len() / 16);
    for byte in data {
        if *byte == b'\n' {
            out.push(b'\r');
        }
        out.push(*byte);
    }
    Cow::Owned(out)
}

/// Waits until `deadline`; a branch with no deadline is off.
async fn sleep_until(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_with_no_terminal_gets_a_carriage_return_before_each_line_feed() {
        assert_eq!(&*lines(b"a\nb\n", false), b"a\r\nb\r\n");
        assert!(matches!(lines(b"a\nb\n", true), Cow::Borrowed(_)), "a terminal sends its own line ends");
        assert!(matches!(lines(b"no line end", false), Cow::Borrowed(_)));
    }
}
