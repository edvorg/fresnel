//! Each end of a socket: its state, its counters, its report, and the last messages.

use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

use super::*;

fn close(code: CloseCode, reason: &str) -> Message {
    Message::Close(Some(CloseFrame { code, reason: reason.into() }))
}

#[tokio::test(start_paused = true)]
async fn a_close_of_the_app_sends_the_waiting_messages_then_the_close_message() {
    let mut bed = Bed::start().await;
    bed.socket.writer().send(Bytes::from_static(b"\x00last")).await.expect("a send");
    bed.socket.close();
    bed.until("closing", |state| *state == SocketState::Closing).await;
    assert!(bed.socket.writer().send(Bytes::from_static(b"\x00late")).await.is_err(), "no message after the close");
    // The socket reads during its close: the last output of an exec arrives.
    bed.peer_sends(b"\x01an answer before the close").await;
    assert_eq!(bed.peer_reads().await, Message::Binary(Bytes::from_static(b"\x00last")));
    assert_eq!(bed.peer_reads().await, close(CloseCode::Normal, ""));
    // tungstenite answers the close at the next read of the peer, which then ends.
    assert!(bed.peer.next().await.is_none(), "the protocol of the peer ends with its answer");
    assert_eq!(bed.socket.recv().await.as_deref(), Some(&b"\x01an answer before the close"[..]));
    assert_eq!(bed.end(Instant::now()).await.0, SocketEnd::ClosedByApp);
    let counts = bed.counts();
    assert_eq!((counts.open, counts.closed, counts.failed, bed.ends()), (0, 1, 0, vec![SocketEnd::ClosedByApp]));
}

#[tokio::test(start_paused = true)]
async fn a_close_with_no_answer_ends_at_its_limit() {
    let bed = Bed::start().await;
    let start = Instant::now();
    bed.socket.close();
    assert_eq!(bed.end(start).await, (SocketEnd::ClosedByApp, 5 * SECOND));
}

/// The close of a server can carry a reason, for example of a pod that went away.
#[tokio::test(start_paused = true)]
async fn the_close_of_the_server_ends_the_socket_with_its_code_and_reason_after_its_messages() {
    let mut bed = Bed::start().await;
    bed.peer_sends(b"\x01before the close").await;
    bed.peer.send(close(CloseCode::Error, "the pod went away")).await.expect("a close");
    assert_eq!(bed.socket.recv().await.as_deref(), Some(&b"\x01before the close"[..]));
    assert_eq!(bed.socket.recv().await, None);
    let end = SocketEnd::ClosedByServer { code: 1011, reason: "the pod went away".to_owned() };
    assert_eq!(bed.end(Instant::now()).await.0, end);
    assert!(!end.is_clean(), "1011 is an error of the server");
    let counts = bed.counts();
    let text = "closed by the server (1011): the pod went away";
    assert_eq!((counts.failed, counts.last_failure.as_deref()), (1, Some(text)));
    assert_eq!(bed.peer_reads().await, close(CloseCode::Error, "the pod went away"), "the socket answers the close");
}

/// The API server ends the connection of an exec after its status, with no close message.
#[tokio::test(start_paused = true)]
async fn the_end_of_the_connection_with_no_close_message_is_a_clean_end_of_the_server() {
    let mut bed = Bed::start().await;
    bed.peer_sends(b"\x03{\"status\":\"Success\"}").await;
    bed.peer_ends().await;
    assert_eq!(bed.socket.recv().await.as_deref(), Some(&b"\x03{\"status\":\"Success\"}"[..]));
    assert_eq!(bed.socket.recv().await, None);
    assert_eq!(bed.end(Instant::now()).await.0, SocketEnd::EndedByServer);
    assert_eq!((bed.counts().closed, bed.ends()), (1, vec![SocketEnd::EndedByServer]));
}

/// Some load balancers end TLS with no close_notify after the close message of the server.
/// The close message ended the protocol, so that is a clean end, not a break.
#[tokio::test(start_paused = true)]
async fn a_close_of_the_server_and_then_an_end_of_tls_with_no_close_notify_is_a_clean_end() {
    let mut bed = Bed::start().await;
    bed.no_close_notify.store(true, Ordering::Relaxed);
    bed.peer_sends(b"\x01the last bytes").await;
    bed.peer.send(close(CloseCode::Normal, "")).await.expect("a close");
    bed.peer_ends().await;
    assert_eq!(bed.socket.recv().await.as_deref(), Some(&b"\x01the last bytes"[..]));
    let end = SocketEnd::ClosedByServer { code: 1000, reason: String::new() };
    assert_eq!(bed.end(Instant::now()).await.0, end);
    assert_eq!((bed.counts().closed, bed.counts().failed), (1, 0));
}

/// The same end with no close message is a break: the data can be cut.
#[tokio::test(start_paused = true)]
async fn an_end_of_tls_with_no_close_notify_and_no_close_message_is_a_break() {
    let mut bed = Bed::start().await;
    bed.no_close_notify.store(true, Ordering::Relaxed);
    bed.peer_ends().await;
    let (end, _) = bed.end(Instant::now()).await;
    assert!(
        matches!(&end, SocketEnd::Failed(SocketFailure::Broken(text)) if text.contains(NO_CLOSE_NOTIFY)),
        "{end:?}"
    );
}

/// Review attack A2 of the terminal prototype: a key after `exit` breaks the write, and the
/// status that the server sent before its end must still arrive.
#[tokio::test(start_paused = true)]
async fn a_failed_write_keeps_the_read_so_the_last_message_of_the_server_arrives() {
    let mut bed = Bed::start().await;
    bed.writes_fail.store(true, Ordering::Relaxed);
    let writer = bed.socket.writer();
    while writer.send(Bytes::from_static(b"\x00a key")).await.is_ok() {}
    bed.peer_sends(b"\x03{\"status\":\"Failure\"}").await;
    bed.peer_ends().await;
    assert_eq!(bed.socket.recv().await.as_deref(), Some(&b"\x03{\"status\":\"Failure\"}"[..]));
    assert_eq!(bed.end(Instant::now()).await.0, SocketEnd::EndedByServer);
}

#[tokio::test(start_paused = true)]
async fn a_failed_write_with_a_silent_peer_breaks_with_its_reason() {
    let bed = Bed::start().await;
    bed.writes_fail.store(true, Ordering::Relaxed);
    let writer = bed.socket.writer();
    let start = Instant::now();
    while writer.send(Bytes::from_static(b"\x00a key")).await.is_ok() {}
    let (end, after) = bed.end(start).await;
    assert_eq!(end, SocketEnd::Failed(SocketFailure::Broken("IO error: broken pipe".to_owned())));
    assert_eq!(after, 5 * SECOND);
}

#[tokio::test(start_paused = true)]
async fn a_text_message_breaks_the_binary_subprotocol() {
    let mut bed = Bed::start().await;
    bed.peer.send(Message::text("a text")).await.expect("a send");
    let text = "the server sent a text message, and the subprotocol is binary";
    assert_eq!(bed.end(Instant::now()).await.0, SocketEnd::Failed(SocketFailure::Invalid(text.to_owned())));
}

#[tokio::test(start_paused = true)]
async fn a_dropped_socket_reports_its_end_and_leaves_the_counters() {
    let Bed { socket, counts, ends, changed, .. } = Bed::start().await;
    let mut changes = changed.watch();
    // The drop aborts the task, and the runtime drops its state at its next turn.
    drop(socket);
    while ends.lock().expect("a lock").is_empty() {
        timeout(SECOND, changes.next()).await.expect("the end in time");
    }
    assert_eq!(*ends.lock().expect("a lock"), [SocketEnd::Dropped]);
    let counts = counts.read();
    assert_eq!((counts.open, counts.opened, counts.closed), (0, 1, 1));
}

/// Design rule 1: a panic of the task gives a final state, a report and the end.
#[tokio::test(start_paused = true)]
async fn a_panic_in_the_task_gives_the_final_state_and_a_report() {
    let bed = Bed::start().await;
    // The path is read at the limit of a ping that waits behind a write.
    bed.path.0.store(PANICS, Ordering::Relaxed);
    let piece = Bytes::from(vec![7_u8; 32 * 1024]);
    let writer = bed.socket.writer();
    let write = async { while writer.send(piece.clone()).await.is_ok() {} };
    let ((end, after), ()) = tokio::join!(bed.end(Instant::now()), write);
    assert_eq!(after, 15 * SECOND);
    match end {
        SocketEnd::Failed(SocketFailure::Panicked(text)) => assert!(text.contains("a defect in the path"), "{text}"),
        other => panic!("a panic: {other:?}"),
    }
    assert_eq!(bed.counts().failed, 1);
    let reports = bed.reports.lock().expect("a lock").join("; ");
    assert!(reports.starts_with("task test socket [memory] panicked: "), "{reports}");
}
