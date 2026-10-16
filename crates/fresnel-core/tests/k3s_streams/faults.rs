//! The faults of a socket on k3s: a pod that reads no input, a reset and a silent connection.

use std::time::{Duration, Instant};

use fresnel_core::net::{SocketEnd, SocketFailure, SocketState};
use fresnel_core::stream::{Exec, ExecEnd, ExecSession, Output, Tunnel, TunnelEnd};

use crate::bed::{Bed, journal, output, pod, sh};
use crate::common::{in_time, until_live};

const SECOND: Duration = Duration::from_secs(1);

/// Plan section 12.3: a write that waits for a slow pod is not a break. The process reads no
/// input. The API server of k3s takes the 64 MiB, which the pipe of the process cannot, and
/// answers no ping behind them: the socket waits for the pod and does not break.
pub(crate) async fn a_pod_that_reads_no_input_makes_the_input_wait_with_no_break(bed: &Bed) {
    let exec = Exec { pod: pod("shell"), container: None, command: sh("sleep 600"), tty: false, stdin: true };
    let session = ExecSession::open(&bed.link.gateway, &exec, journal()).await.expect("an exec");
    let failed = bed.link.net.status().upgrade.sockets.failed;
    let (input, mut states) = (session.input(), session.states());
    let sent = vec![b'x'; 64 * 1024 * 1024];
    let write = async { input.send(&sent).await };
    let watch = async {
        let waits = states.wait_for(|state| matches!(state, SocketState::WriteWaits { .. }));
        in_time("a write that waits", waits).await.expect("the socket");
        // Four pings with no answer while the cluster answers on its channels.
        tokio::time::sleep(4 * SECOND).await;
        assert!(matches!(*states.borrow(), SocketState::WriteWaits { .. }), "{:?}", *states.borrow());
        assert_eq!(bed.link.net.status().upgrade.sockets.failed, failed, "a pod that reads nothing is no break");
        session.close();
        in_time("the end", states.wait_for(|state| matches!(state, SocketState::Ended(_)))).await.expect("the socket");
    };
    let (written, ()) = tokio::join!(write, watch);
    // The input went into the buffers of the server, or the close ended its wait.
    if let Err(error) = written {
        assert_eq!(error.to_string(), "the WebSocket ended");
    }
    assert_eq!(session.end(), Some(ExecEnd::Socket(SocketEnd::ClosedByApp)));
}

pub(crate) async fn a_silent_connection_and_a_reset_end_their_sockets_with_the_reason(bed: &Bed) {
    // The web server waits for the request of the tunnel.
    let mut tunnel = Tunnel::open(&bed.link.gateway, &pod("web"), 8080, journal()).await.expect("a tunnel");
    // On macOS the proxy waits up to 2 s for each peer to take its bytes, then resets.
    in_time("the reset", bed.proxy.reset_all()).await;
    let reset = Instant::now();
    assert_eq!(in_time("the end of the tunnel", tunnel.next()).await, None);
    match tunnel.end() {
        Some(TunnelEnd::Socket(SocketEnd::Failed(SocketFailure::Broken(_)))) => {}
        other => panic!("a broken socket: {other:?}"),
    }
    assert!(reset.elapsed() < SECOND, "the reset shows at once: {:?}", reset.elapsed());
    until_live(&bed.link).await;

    let cat =
        Exec { pod: pod("shell"), container: None, command: Some(vec!["cat".to_owned()]), tty: false, stdin: true };
    let mut session = ExecSession::open(&bed.link.gateway, &cat, journal()).await.expect("an exec");
    session.input().send(b"ready\n").await.expect("the input");
    assert!(matches!(in_time("the echo", session.next()).await, Some(Output::Stdout(_))));
    let silenced = Instant::now();
    bed.proxy.silence_open();
    output(&mut session).await;
    match session.end() {
        Some(ExecEnd::Socket(SocketEnd::Failed(SocketFailure::Silent { .. }))) => {}
        other => panic!("a silent socket: {other:?}"),
    }
    // A ping after 1 s and 1 s for its answer, on a loaded machine.
    assert!(silenced.elapsed() < 4 * SECOND, "the silence shows in its limit: {:?}", silenced.elapsed());
}
