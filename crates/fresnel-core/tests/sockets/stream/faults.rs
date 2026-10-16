//! The faults of a socket: a silent connection, a reset, held connects, and a pod that reads
//! nothing. The pings of these tests are short; the unit tests of `fresnel-net` prove the
//! default times with paused time.

use std::time::Instant;

use fresnel_core::net::{ErrorClass, Limits, SocketEnd, SocketFailure, SocketState};
use fresnel_core::stream::ExecEnd;
use fresnel_testkit::ProxyMode;

use super::*;
use crate::bed::ProxyBed;

/// The end of a session that read its output to the end.
fn socket_end(session: &ExecSession) -> SocketEnd {
    match session.end() {
        Some(ExecEnd::Socket(end)) => end,
        other => panic!("an end of the socket: {other:?}"),
    }
}

/// Opens a shell and proves that its bytes flow.
async fn flowing(gateway: &fresnel_core::cluster::Gateway, journal: &Journal) -> ExecSession {
    let mut session = ExecSession::open(gateway, &sh(), journal.clone()).await.expect("an exec");
    session.input().send(b"echo ready\n").await.expect("the input");
    assert!(matches!(in_time("the echo", session.next()).await, Some(Output::Stdout(_))));
    session
}

/// Plan section 15, M5 and M10: a silent exec connection shows in 20 s or less; here the
/// ping comes after 1 s and has 1 s for its answer.
#[tokio::test]
async fn a_silent_connection_ends_the_session_at_the_limit_of_its_ping_with_an_entry() {
    let bed = ProxyBed::start("silent-socket", short_pings()).await;
    let journal = journal();
    let mut session = flowing(&bed.link.gateway, &journal).await;
    let silenced = Instant::now();
    bed.proxy.silence_open();
    assert_eq!(output(&mut session).await, (Vec::new(), Vec::new()));
    let after = silenced.elapsed();
    let SocketEnd::Failed(SocketFailure::Silent { quiet }) = socket_end(&session) else { panic!("silent") };
    assert!(quiet >= 2 * SECOND && after < 3 * SECOND, "the break after {after:?}, quiet {quiet:?}");
    let entry = entries(&journal).pop().expect("an entry");
    let start = "Warning bed: an exec in pod default/web: the WebSocket ended: silent: no frame for ";
    assert!(entry.starts_with(start), "{entry}");
    let sockets = bed.link.net.status().upgrade.sockets;
    assert_eq!((sockets.failed, sockets.silent), (1, 1));
}

#[tokio::test]
async fn a_reset_ends_the_session_at_once_with_its_reason() {
    let bed = ProxyBed::start("reset-socket", NetSettings::default()).await;
    let mut session = flowing(&bed.link.gateway, &journal()).await;
    // On macOS the proxy waits for each peer to take its bytes, and then resets.
    bed.proxy.reset_all().await;
    let reset = Instant::now();
    assert_eq!(output(&mut session).await, (Vec::new(), Vec::new()));
    let end = socket_end(&session);
    assert!(matches!(end, SocketEnd::Failed(SocketFailure::Broken(_))), "{end:?}");
    assert!(reset.elapsed() < SECOND, "the reset shows at once: {:?}", reset.elapsed());
}

/// Each new connection gets no answer: a session opens on a warm connection, and with no
/// warm connection its open fails at the time limit of an upgrade.
#[tokio::test]
async fn held_connects_cost_a_session_nothing_on_a_warm_connection_and_its_limit_without_one() {
    let limits = Limits { upgrade: 2 * SECOND, ..Limits::default() };
    let bed = ProxyBed::start("held-socket", NetSettings { limits, ..NetSettings::default() }).await;
    let terminal = bed.link.net.keep_upgrades_warm();
    until(&bed.link, "two warm connections", |status| status.upgrade.warm == 2).await;
    bed.proxy.set(ProxyMode::HoldNew { probability: 1.0, min: 60 * SECOND, max: 60 * SECOND });
    let opened = Instant::now();
    let mut session = flowing(&bed.link.gateway, &journal()).await;
    assert!(opened.elapsed() < SECOND, "a warm connection waits for no connect: {:?}", opened.elapsed());
    assert_eq!(bed.link.net.status().upgrade.reused, 1);
    session.input().send(b"exit 0\n").await.expect("the input");
    assert_eq!(output(&mut session).await.0, b"exit 0\n");

    drop(terminal);
    until(&bed.link, "no warm connection", |status| status.upgrade.warm == 0).await;
    let journal = journal();
    let error = ExecSession::open(&bed.link.gateway, &sh(), journal.clone()).await.expect_err("each connect is held");
    assert_eq!(error.class(), ErrorClass::Timeout, "{error}");
    assert_eq!(entries(&journal).len(), 1, "the journal tells why it did not start");
}

/// Plan section 12.3: a write that waits for a slow pod is not a break. The pod reads
/// nothing for some pings while the channels of the cluster stay live.
#[tokio::test]
async fn a_pod_that_reads_nothing_makes_the_input_wait_with_no_break() {
    let bed = Bed::start(short_pings()).await;
    let mut session = ExecSession::open(&bed.link.gateway, &sh(), journal()).await.expect("an exec");
    bed.server.hold_streams(true);
    let input = session.input();
    let sent = vec![b'x'; 2 * 1024 * 1024];
    let mut states = session.states();
    let write = async { input.send(&sent).await.expect("the input") };
    let wait = async {
        let waits = states.wait_for(|state| matches!(state, SocketState::WriteWaits { .. }));
        in_time("the state of a write that waits", waits).await.expect("the socket");
        // Pings with no answer: the cluster answers, so the wait is no break.
        tokio::time::sleep(4 * SECOND).await;
        assert!(matches!(*states.borrow(), SocketState::WriteWaits { .. }), "{:?}", *states.borrow());
        bed.server.hold_streams(false);
    };
    let read = async {
        let mut echoed = 0;
        while echoed < sent.len() {
            match in_time("the echo", session.next()).await {
                Some(Output::Stdout(data)) => echoed += data.len(),
                other => panic!("the echo of the input: {other:?}"),
            }
        }
    };
    tokio::join!(write, wait, read);
    in_time("open again", states.wait_for(|state| *state == SocketState::Open)).await.expect("the socket");
    let sockets = bed.link.net.status().upgrade.sockets;
    assert_eq!((sockets.failed, sockets.write_waits), (0, 0));
}
