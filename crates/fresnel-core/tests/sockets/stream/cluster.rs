//! The sockets of one cluster on the parts of the cluster: its signal and its channels.

use std::time::Instant;

use fresnel_core::net::{SocketEnd, SocketFailure, SocketState};

use super::*;
use crate::bed::ProxyBed;

/// Short pings of the sockets. Each lane of channels pings at its own time, as in the app,
/// where each one connected at its own time: after 1, 3 and 5 s, each with 1 s for the answer.
fn staggered_channel_pings() -> NetSettings {
    let mut settings = short_pings();
    for (lane, after) in [(&mut settings.watches, 1), (&mut settings.requests, 3), (&mut settings.logs, 5)] {
        (lane.keepalive_interval, lane.keepalive_timeout) = (after * SECOND, SECOND);
    }
    settings
}

/// Plan section 12.3: a write that waits for a slow pod is no break while a channel of the
/// cluster answers. On a silent path each channel connects again on a new connection, at the
/// time of its own ping; a new connection proves a new path and not the one of the socket. So
/// the socket breaks when the last channel that was live before its silence goes.
#[tokio::test]
async fn a_write_that_waits_breaks_on_a_silent_path_also_when_the_channels_connect_again() {
    let bed = ProxyBed::start("path-socket", staggered_channel_pings()).await;
    let session = ExecSession::open(&bed.link.gateway, &sh(), journal()).await.expect("an exec");
    bed.server.hold_streams(true);
    let (input, mut states) = (session.input(), session.states());
    let sent = vec![b'x'; 2 * 1024 * 1024];
    let write = async { input.send(&sent).await };
    let watch = async {
        let waits = states.wait_for(|state| matches!(state, SocketState::WriteWaits { .. }));
        in_time("a write that waits", waits).await.expect("the socket");
        // Pings with no answer: the channels answer theirs, so the wait is no break.
        tokio::time::sleep(3 * SECOND).await;
        assert!(matches!(*states.borrow(), SocketState::WriteWaits { .. }), "{:?}", *states.borrow());
        let silenced = Instant::now();
        bed.proxy.silence_open();
        let ended = states.wait_for(|state| matches!(state, SocketState::Ended(_)));
        let end = in_time("the end of the socket", ended).await.expect("the socket").clone();
        (end, silenced.elapsed())
    };
    let (written, (end, after)) = tokio::join!(write, watch);
    // The input went into the buffers of the connection, or the end of the socket ended its wait.
    if let Err(error) = written {
        assert_eq!(error.to_string(), "the WebSocket ended");
    }
    assert!(matches!(end, SocketState::Ended(SocketEnd::Failed(SocketFailure::Silent { .. }))), "{end:?}");
    // The last lane pings after 5 s and has 1 s for the answer, on a loaded machine.
    assert!(after < 9 * SECOND, "the break after the channels from before the silence: {after:?}");
    let channels = bed.link.net.status().watches.channels;
    assert!(channels.iter().any(|channel| channel.connections >= 2), "the channels connected again: {channels:?}");
}
