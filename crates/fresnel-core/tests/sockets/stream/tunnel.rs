//! The tunnel of a port forward: the bytes of a port in each direction, and a closed port.

use fresnel_core::net::{SocketEnd, SocketState};
use fresnel_core::stream::{Tunnel, TunnelEnd};
use http::header::SEC_WEBSOCKET_PROTOCOL;

use super::*;

/// More bytes than the queues of the socket and the buffers of the connection hold, so the
/// write waits for the read and each piece passes the bounded queues in its sequence.
#[tokio::test]
async fn a_tunnel_carries_the_bytes_of_a_port_in_each_direction_in_their_sequence() {
    let bed = Bed::start(NetSettings::default()).await;
    let mut tunnel = Tunnel::open(&bed.link.gateway, &pod(), 8080, journal()).await.expect("a tunnel");
    let sent: Vec<u8> = (0..4 * 1024 * 1024).map(|byte: u32| byte.to_le_bytes()[1]).collect();
    let writer = tunnel.writer();
    let write = async { writer.send(&sent).await.expect("the data") };
    let read = async {
        let mut back = Vec::new();
        while back.len() < sent.len() {
            back.extend_from_slice(&in_time("the data of the port", tunnel.next()).await.expect("data"));
        }
        back
    };
    let ((), back) = tokio::join!(write, read);
    assert!(back == sent, "each byte comes back in its sequence");

    tunnel.close();
    assert_eq!(in_time("the end", tunnel.next()).await, None);
    assert_eq!(tunnel.end(), Some(TunnelEnd::Socket(SocketEnd::ClosedByApp)));
    let requests = bed.server.requests();
    let seen = requests.iter().find(|seen| seen.uri.path().ends_with("/portforward")).expect("the request");
    // kube starts the query with `&`, and the API server takes it.
    assert_eq!(seen.uri.query(), Some("&ports=8080"));
    assert_eq!(seen.headers[SEC_WEBSOCKET_PROTOCOL], "v4.channel.k8s.io");
    // 128 pieces of 32 KiB, each with its channel byte, and the port at the start of each
    // of the two channels of the server.
    let sockets = closed_sockets(&bed).await;
    let pieces = 4 * 1024 * 1024 + 128;
    assert_eq!((sockets.closed, sockets.sent, sockets.received), (1, pieces, pieces + 2 * 3));
}

/// A node writes the failure of its dial on the error channel and ends the connection. A
/// forward needs the text: three of them in sequence make a port that is closed.
#[tokio::test]
async fn a_closed_port_ends_the_tunnel_with_the_text_of_the_node() {
    let bed = Bed::start(NetSettings::default()).await;
    let journal = journal();
    let mut tunnel = Tunnel::open(&bed.link.gateway, &pod(), 1, journal.clone()).await.expect("a tunnel");
    assert_eq!(in_time("the end", tunnel.next()).await, None);
    let text = "error forwarding port 1 to pod web: dial tcp4 127.0.0.1:1: connection refused";
    assert_eq!(tunnel.end(), Some(TunnelEnd::Refused(text.to_owned())));
    assert_eq!(entries(&journal), Vec::<String>::new(), "a tunnel has no entry of its own: its forward has");
}

/// Plan section 15, M5: an upload to a slow pod completes. The pod reads in short bursts with
/// pauses longer than the limit of a ping, while the channels of the cluster answer. A pod of
/// the fake server that pauses reads nothing at all, also no ping, so the pauses end with the
/// upload: a real one reads on until its buffers are full.
#[tokio::test]
async fn an_upload_to_a_pod_that_reads_in_bursts_completes_with_no_break() {
    let bed = Bed::start(short_pings()).await;
    let mut tunnel = Tunnel::open(&bed.link.gateway, &pod(), 8080, journal()).await.expect("a tunnel");
    let sent: Vec<u8> = (0..16 * 1024 * 1024).map(|byte: u32| byte.to_le_bytes()[2]).collect();
    let writer = tunnel.writer();
    let write = async { writer.send(&sent).await.expect("the data") };
    let waited = std::cell::Cell::new(0);
    let slow_pod = async {
        loop {
            bed.server.hold_streams(true);
            tokio::time::sleep(3 * SECOND).await;
            waited.set(waited.get() + bed.link.net.status().upgrade.sockets.write_waits);
            bed.server.hold_streams(false);
            tokio::time::sleep(SECOND / 20).await;
        }
    };
    let read = async {
        let mut back = Vec::new();
        while back.len() < sent.len() {
            back.extend_from_slice(&in_time("the data of the port", tunnel.next()).await.expect("data"));
        }
        back
    };
    let until_read = async {
        tokio::select! {
            back = read => back,
            () = slow_pod => unreachable!("the pod pauses until the end of the read"),
        }
    };
    let ((), back) = tokio::join!(write, until_read);
    bed.server.hold_streams(false);
    assert!(back == sent, "each byte comes back in its sequence");
    assert!(waited.get() >= 2, "the pauses of the pod made the write wait: {}", waited.get());
    assert_eq!((tunnel.state(), bed.link.net.status().upgrade.sockets.failed), (SocketState::Open, 0));
}
