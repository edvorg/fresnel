//! Exec on k3s: the channels, the exit codes, the size of a terminal and the refusals.

use fresnel_core::net::ErrorClass;
use fresnel_core::stream::{Exec, ExecEnd, ExecSession, Exit, Output};

use crate::bed::{Bed, journal, output, pod, sh};
use crate::common::in_time;

/// A shell that reads a line, answers on each output and ends with 3; then `cat`, whose
/// input ends with the close signal of v5.
pub(crate) async fn the_channels_carry_each_byte_and_the_status_gives_the_exit_code(bed: &Bed) {
    let gateway = &bed.link.gateway;
    let journal = journal();
    let script = "read line; echo \"got $line\"; echo oops >&2; exit 3";
    let exec = Exec { pod: pod("shell"), container: None, command: sh(script), tty: false, stdin: true };
    let mut session = ExecSession::open(gateway, &exec, journal.clone()).await.expect("an exec");
    session.input().send(b"hello\n").await.expect("the input");
    assert_eq!(output(&mut session).await, (b"got hello\n".to_vec(), b"oops\n".to_vec()));
    assert_eq!(session.end(), Some(ExecEnd::Exited(Exit::Code(3))));
    let texts: Vec<String> = journal.view().entries.iter().map(|entry| entry.text.clone()).collect();
    assert_eq!(
        texts,
        [
            "an exec in pod default/shell: open, v5.channel.k8s.io",
            "an exec in pod default/shell: the process ended with the exit code 3",
        ]
    );

    let cat = Exec { command: Some(vec!["cat".to_owned()]), ..exec };
    let mut session = ExecSession::open(gateway, &cat, journal.clone()).await.expect("an exec of cat");
    let sent: Vec<u8> = (0..256 * 1024).map(|byte: u32| b'a' + (byte % 26) as u8).collect();
    let input = session.input();
    let write = async {
        input.send(&sent).await.expect("the input");
        input.close_stdin().await.expect("the end of the input");
    };
    let ((), (stdout, stderr)) = tokio::join!(write, output(&mut session));
    assert!(stdout == sent && stderr.is_empty(), "cat gives each byte back: {} of {}", stdout.len(), sent.len());
    assert_eq!(session.end(), Some(ExecEnd::Exited(Exit::Code(0))));
}

/// The size reaches the terminal of the process. A program that does not exist ends with
/// the reason of the server, and a container that does not exist is a refusal with its message.
pub(crate) async fn a_terminal_gets_its_size_and_a_missing_program_or_container_tells_why(bed: &Bed) {
    let gateway = &bed.link.gateway;
    let terminal = Exec { pod: pod("shell"), container: None, command: sh("exec sh"), tty: true, stdin: true };
    let mut session = ExecSession::open(gateway, &terminal, journal()).await.expect("a terminal");
    let input = session.input();
    input.resize(132, 43).await.expect("the size");
    // The server applies a size on an other stream than the input, so a read can come first.
    let mut screen = Vec::new();
    while !String::from_utf8_lossy(&screen).contains("43 132") {
        input.send(b"stty size\n").await.expect("the input");
        if let Some(Output::Stdout(data)) = in_time("the size of the terminal", session.next()).await {
            screen.extend_from_slice(&data);
        }
    }
    input.send(b"exit 5\n").await.expect("the input");
    output(&mut session).await;
    assert_eq!(session.end(), Some(ExecEnd::Exited(Exit::Code(5))));

    let missing = Exec { command: Some(vec!["no-such-program".to_owned()]), tty: false, ..terminal.clone() };
    let mut session = ExecSession::open(gateway, &missing, journal()).await.expect("an exec");
    output(&mut session).await;
    match session.end() {
        Some(ExecEnd::Exited(Exit::Error { reason, message })) => {
            assert_eq!(reason, "InternalError");
            assert!(message.contains("no-such-program"), "{message}");
        }
        other => panic!("the reason of the server: {other:?}"),
    }

    let other_container = Exec { container: Some("nope".to_owned()), ..terminal };
    let error = ExecSession::open(gateway, &other_container, journal()).await.expect_err("no such container");
    assert_eq!((error.class(), error.status()), (ErrorClass::Invalid, Some(400)));
    assert!(error.to_string().contains("nope"), "the message of the server: {error}");
}
