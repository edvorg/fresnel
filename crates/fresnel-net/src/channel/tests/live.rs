//! The state "live": only an answer shows that a connection works (plan section 8.5).

use super::*;

#[tokio::test(start_paused = true)]
async fn a_channel_is_live_only_after_an_answer() {
    let server = MemoryServer::new();
    server.set_mute(true);
    let mut bed = Bed::on(server, settings());
    bed.until_state(ChannelState::Verifying).await;
    assert_eq!((bed.server.dials(), bed.elapsed()), (1, Duration::ZERO), "the connection is open at once");

    let status = bed.until("backoff", |status| matches!(status.state, ChannelState::Backoff { .. })).await;
    assert_eq!(bed.elapsed(), 10 * SECOND, "the first answer has a time limit");
    assert_eq!(status.last_error.as_deref(), Some("no answer to the light request in 10s"));
    assert_eq!((status.connections, status.server_stream_limit, status.ping_time), (1, None, None));

    bed.server.set_mute(false);
    let live = bed.until_state(ChannelState::Live).await;
    assert_eq!((live.connections, live.server_stream_limit, live.stream_budget), (2, Some(100), 70));
    assert_eq!(live.ping_time, Some(Duration::ZERO));
}

/// The light request of a new connection can wait at the server while a request of a
/// caller gets its answer. That answer shows that the connection works.
#[tokio::test(start_paused = true)]
async fn the_answer_to_a_request_of_a_caller_makes_the_channel_live() {
    let server = MemoryServer::new();
    server.mute_path("/version");
    let mut bed = Bed::on(server, settings());
    bed.until_state(ChannelState::Verifying).await;
    sleep(SECOND).await;
    assert_eq!(bed.status().state, ChannelState::Verifying, "the light request got no answer");

    assert!(bed.open("/api/v1/pods").await.is_ok());
    let live = bed.until_state(ChannelState::Live).await;
    assert_eq!((bed.elapsed(), live.connections), (SECOND, 1));
    assert_eq!((live.server_stream_limit, live.ping_time), (Some(100), None), "no light request gave a time");
}
