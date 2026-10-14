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

/// The detectors can tell a change each 2 s. A raise during the test of a connection does
/// not give the test a new limit, so a silent connection ends 3 s after the first raise,
/// and not at the ping 15 s after its last frame.
#[tokio::test(start_paused = true)]
async fn a_raise_each_2_seconds_does_not_keep_a_silent_connection_in_its_test() {
    let mut bed = Bed::start(settings());
    bed.until_state(ChannelState::Live).await;
    let _stream = bed.open("/watch/pods").await.expect("a stream");
    bed.server.silence_open();
    let silent_at = bed.elapsed();
    let signal = bed.signal.clone();
    let storm = async move {
        for _ in 0..10 {
            signal.raise(Cause::NetworkChange);
            sleep(2 * SECOND).await;
        }
    };
    let lost = async {
        let status = bed.until("the loss of the connection", |status| status.last_error.is_some()).await;
        (status, bed.elapsed())
    };
    let ((status, lost_at), ()) = tokio::join!(lost, storm);
    assert_eq!(lost_at - silent_at, 3 * SECOND, "{:?}", status.last_error);
    assert_eq!(status.last_error.as_deref(), Some("no answer to the light request in 3s"));
}
