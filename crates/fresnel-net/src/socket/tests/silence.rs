//! The ping of a socket: a silent connection breaks in its limit, a good one never.

use super::*;
use crate::signal::Cause;

fn silent(quiet: Duration) -> SocketEnd {
    SocketEnd::Failed(SocketFailure::Silent { quiet })
}

/// Plan section 15, M5 and M10: a silent connection shows in 20 s or less.
#[tokio::test(start_paused = true)]
async fn a_silent_connection_breaks_15_s_after_its_last_frame() {
    let mut bed = Bed::start().await;
    bed.peer_sends(b"\x01last frame").await;
    let start = Instant::now();
    assert_eq!(bed.socket.recv().await.as_deref(), Some(&b"\x01last frame"[..]));
    // From now on the peer reads nothing and answers no ping.
    assert_eq!(bed.end(start).await, (silent(15 * SECOND), 15 * SECOND));
    assert_eq!(bed.socket.recv().await, None, "the owner reads the end of the messages");
    let counts = bed.counts();
    assert_eq!((counts.open, counts.failed, counts.silent, counts.closed), (0, 1, 1, 0));
    let text = "silent: no frame for 15.0 s, also no answer to a ping";
    assert_eq!((counts.last_failure.as_deref(), bed.ends()), (Some(text), vec![silent(15 * SECOND)]));
}

#[tokio::test(start_paused = true)]
async fn a_peer_that_answers_each_ping_keeps_a_quiet_socket_open() {
    let mut bed = Bed::start().await;
    let mut pings = 0;
    let answer = async {
        while let Some(Ok(message)) = bed.peer.next().await {
            pings += u32::from(matches!(message, Message::Ping(_)));
        }
    };
    assert!(timeout(65 * SECOND, answer).await.is_err(), "the peer reads to the end of the test");
    assert_eq!(pings, 6, "one ping each 10 s with no other frame");
    assert_eq!((bed.socket.state(), bed.ends()), (SocketState::Open, Vec::new()));
}

/// The cluster stops to answer while a write waits: the wait was no proof of a live
/// connection, and the change of the path ends it at once.
#[tokio::test(start_paused = true)]
async fn a_write_that_waits_breaks_when_the_cluster_does_not_answer_either() {
    let bed = Bed::start().await;
    let start = Instant::now();
    let piece = Bytes::from(vec![7_u8; 32 * 1024]);
    let writer = bed.socket.writer();
    let write = async { while writer.send(piece.clone()).await.is_ok() {} };
    let fault = async {
        tokio::time::sleep(40 * SECOND).await;
        bed.set_path(DOWN);
    };
    let (end, (), ()) = tokio::join!(bed.end(start), write, fault);
    assert_eq!(end, (silent(40 * SECOND), 40 * SECOND));
}

#[tokio::test(start_paused = true)]
async fn a_write_that_waits_on_a_cluster_with_no_live_channel_breaks_at_the_limit_of_its_ping() {
    let bed = Bed::start().await;
    bed.path.0.store(DOWN, Ordering::Relaxed);
    let start = Instant::now();
    let piece = Bytes::from(vec![7_u8; 32 * 1024]);
    let writer = bed.socket.writer();
    let write = async { while writer.send(piece.clone()).await.is_ok() {} };
    let (end, ()) = tokio::join!(bed.end(start), write);
    assert_eq!(end, (silent(15 * SECOND), 15 * SECOND));
}

/// A pod that reads no input answers no ping, also when the buffers of the system took each
/// byte: the ping counts the bytes before it, and the answer to a later ping confirms them.
#[tokio::test(start_paused = true)]
async fn data_that_the_peer_took_and_did_not_read_is_no_silence_while_the_cluster_answers() {
    let mut bed = Bed::with_pipe(SocketSettings::default(), 4 * 1024 * 1024).await;
    let piece = Bytes::from(vec![7_u8; 32 * 1024]);
    for _ in 0..32 {
        bed.socket.writer().send(piece.clone()).await.expect("the pipe takes each piece");
    }
    tokio::time::sleep(60 * SECOND).await;
    assert!(matches!(bed.socket.state(), SocketState::WriteWaits { .. }), "{:?}", bed.socket.state());
    assert_eq!(bed.ends(), Vec::new(), "a peer that reads nothing is no break");

    // The peer reads each piece and answers the ping, which confirms them.
    for _ in 0..32 {
        assert_eq!(bed.peer_reads().await, Message::Binary(piece.clone()));
    }
    let answered = async {
        while !matches!(bed.socket.state(), SocketState::Open) {
            match timeout(SECOND, bed.peer.next()).await {
                Ok(Some(Ok(Message::Ping(_)))) | Err(_) => {}
                other => panic!("a ping of the socket: {other:?}"),
            }
        }
    };
    timeout(60 * SECOND, answered).await.expect("open again in time");
    // The confirmed bytes do not excuse a silence: it breaks 15 s after the answer.
    let (end, after) = bed.end(Instant::now()).await;
    assert_eq!(end, silent(15 * SECOND));
    assert!(after <= 15 * SECOND, "{after:?}");
}

/// The keys of a terminal are far below the bytes that a peer reads before it can stop.
#[tokio::test(start_paused = true)]
async fn a_few_keys_with_no_answer_are_a_silent_connection() {
    let bed = Bed::start().await;
    let start = Instant::now();
    bed.socket.writer().send(Bytes::from_static(b"\x00ls -l\n")).await.expect("a key");
    assert_eq!(bed.end(start).await, (silent(15 * SECOND), 15 * SECOND));
}

/// Plan section 8.12: after a wake or a network change, each connection proves itself in 3 s.
#[tokio::test(start_paused = true)]
async fn a_wake_tests_the_connection_with_its_short_limit() {
    let bed = Bed::start().await;
    tokio::time::sleep(4 * SECOND).await;
    let woke = Instant::now();
    bed.signal.raise(Cause::Wake);
    assert_eq!(bed.end(woke).await, (silent(7 * SECOND), 3 * SECOND));
}
