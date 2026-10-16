//! Messages in each direction, the bounded queues, and a write that waits for the peer.

use super::*;

#[tokio::test(start_paused = true)]
async fn messages_go_both_ways_in_their_sequence_and_the_cluster_counts_them() {
    let mut bed = Bed::start().await;
    let writer = bed.socket.writer();
    writer.send(Bytes::from_static(b"\x00one")).await.expect("a send");
    writer.send(Bytes::from_static(b"\x00two")).await.expect("a send");
    assert_eq!(bed.peer_reads().await, Message::Binary(Bytes::from_static(b"\x00one")));
    assert_eq!(bed.peer_reads().await, Message::Binary(Bytes::from_static(b"\x00two")));
    bed.peer_sends(b"\x01out").await;
    assert_eq!(bed.socket.recv().await.as_deref(), Some(&b"\x01out"[..]));
    let counts = bed.counts();
    assert_eq!((counts.open, counts.opened, counts.sent, counts.received), (1, 1, 8, 4));
    assert_eq!((bed.socket.state(), bed.socket.protocol()), (SocketState::Open, "v5.channel.k8s.io"));
}

/// Plan section 12.3: a write that waits for a slow pod is not a break. The peer reads
/// nothing for a minute, so the pipe, the sink and the queue fill and the writer waits.
#[tokio::test(start_paused = true)]
async fn a_write_that_the_peer_does_not_take_waits_with_no_break_while_the_cluster_answers() {
    let mut bed = Bed::start().await;
    let start = Instant::now();
    let piece = Bytes::from(vec![7_u8; 32 * 1024]);
    // 1.25 MiB: more than the pipe, the buffer of the sink and the queue hold together.
    let pieces = 40;
    let writer = bed.socket.writer();
    let write = async {
        for _ in 0..pieces {
            writer.send(piece.clone()).await.expect("a send");
        }
    };
    tokio::pin!(write);
    assert!(timeout(60 * SECOND, &mut write).await.is_err(), "the writer waits for the peer");
    match bed.socket.state() {
        SocketState::WriteWaits { .. } => {}
        other => panic!("the state tells the wait: {other:?}"),
    }
    assert_eq!((bed.counts().write_waits, bed.ends()), (1, Vec::new()), "a wait is no break");

    // The peer reads again: each byte arrives in its sequence, and the writer ends.
    let read = async {
        let mut bytes = 0;
        while bytes < pieces * piece.len() {
            match bed.peer_reads().await {
                Message::Binary(data) if data == piece => bytes += data.len(),
                other => panic!("a piece of the writer: {other:?}"),
            }
        }
    };
    tokio::join!(write, read);
    bed.until("open again", |state| *state == SocketState::Open).await;
    assert_eq!(bed.counts().write_waits, 0);
    assert!(start.elapsed() >= 60 * SECOND);
}

/// A slow owner stops the read of the socket, so the server slows down, and no message is
/// lost. The time with no read is no silence of the connection.
#[tokio::test(start_paused = true)]
async fn a_full_queue_of_the_owner_stops_the_read_and_is_no_silence() {
    let settings = SocketSettings { queue: 1, ..SocketSettings::default() };
    let mut bed = Bed::with(settings).await;
    for message in [b"\x01a", b"\x01b", b"\x01c", b"\x01d", b"\x01e"] {
        bed.peer_sends(message).await;
    }
    tokio::time::sleep(60 * SECOND).await;
    assert_eq!((bed.socket.state(), bed.ends()), (SocketState::Open, Vec::new()));

    let mut read = Vec::new();
    for _ in 0..5 {
        read.push(bed.socket.recv().await.expect("a message"));
    }
    assert_eq!(read, [&b"\x01a"[..], b"\x01b", b"\x01c", b"\x01d", b"\x01e"]);
    // The count of the silence starts at the end of the pause.
    let resumed = Instant::now();
    let (end, after) = bed.end(resumed).await;
    assert_eq!((end, after), (SocketEnd::Failed(SocketFailure::Silent { quiet: 15 * SECOND }), 15 * SECOND));
}
