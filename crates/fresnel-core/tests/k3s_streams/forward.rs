//! The port forward on k3s: a tunnel to a web server in a pod, and to a port with no server.

use fresnel_core::stream::{Tunnel, TunnelEnd};

use crate::bed::{Bed, journal, pod};
use crate::common::in_time;

pub(crate) async fn a_tunnel_reaches_a_web_server_in_a_pod_and_a_port_with_no_server_tells_why(bed: &Bed) {
    let gateway = &bed.link.gateway;
    let mut tunnel = Tunnel::open(gateway, &pod("web"), 8080, journal()).await.expect("a tunnel");
    tunnel.writer().send(b"GET /index.html HTTP/1.0\r\n\r\n").await.expect("the request");
    let mut answer = Vec::new();
    // The server closes its connection after the answer, and the node ends the socket.
    while let Some(data) = in_time("the answer of the web server", tunnel.next()).await {
        answer.extend_from_slice(&data);
    }
    let answer = String::from_utf8_lossy(&answer);
    assert!(answer.contains(" 200 OK") && answer.ends_with("fresnel-forward\n"), "{answer}");
    match tunnel.end() {
        Some(TunnelEnd::Socket(end)) => assert!(end.is_clean(), "{end}"),
        other => panic!("the end of the connection of the pod: {other:?}"),
    }

    let mut tunnel = Tunnel::open(gateway, &pod("web"), 8081, journal()).await.expect("a tunnel");
    assert_eq!(in_time("the end", tunnel.next()).await, None);
    match tunnel.end() {
        Some(TunnelEnd::Refused(text)) => assert!(text.contains("connection refused"), "{text}"),
        other => panic!("the text of the node: {other:?}"),
    }
}
