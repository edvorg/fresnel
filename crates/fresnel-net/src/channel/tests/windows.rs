//! The stream budget and the fixed windows (plan sections 8.6 and 8.7).

use super::*;

const KIB: u32 = 1024;

#[test]
fn one_stream_window_of_the_connection_window_stays_free() {
    // The last one has a connection window that is not a full count of stream windows.
    let odd = ChannelSettings { stream_window: 64 * KIB, connection_window: 200 * KIB, ..settings() };
    let all = [ChannelSettings::for_watches(), ChannelSettings::for_requests(), ChannelSettings::for_logs(), odd];
    for settings in &all {
        for server_limit in [1, 2, 100, 250, 1000, 1_000_000, usize::MAX] {
            let budget = u64::try_from(settings.budget(server_limit)).expect("a small number");
            assert!(budget >= 1);
            assert!(
                (budget + 1) * u64::from(settings.stream_window) <= u64::from(settings.connection_window),
                "limit {server_limit}: {budget} streams of {} bytes and one more do not fit",
                settings.stream_window
            );
        }
    }
    let budgets = all.map(|settings| settings.budget(100));
    assert_eq!(budgets, [70, 70, 70, 2], "a kube-apiserver permits 100 streams");
}

#[tokio::test(start_paused = true)]
async fn at_the_stream_budget_one_more_connection_opens_and_then_a_request_waits() {
    let server = MemoryServer::new();
    server.set_stream_limit(10);
    let mut bed = Bed::on(server, ChannelSettings { extra_idle: 60 * SECOND, ..settings() });
    let live = bed.until_state(ChannelState::Live).await;
    assert_eq!((live.server_stream_limit, live.stream_budget), (Some(10), 7), "70 percent of the limit of the server");

    let mut streams = Vec::new();
    for _ in 0..21 {
        let stream = timeout(5 * SECOND, bed.open("/watch/pods")).await.expect("a free stream on some connection");
        streams.push(stream.expect("a stream"));
    }
    let status = bed.group.status();
    let open: Vec<usize> = status.channels.iter().map(|channel| channel.streams_open).collect();
    assert_eq!((open, status.waiting, bed.server.dials()), (vec![7, 7, 7], 0, 3), "three connections at most");

    let waits = timeout(5 * SECOND, bed.open("/watch/pods")).await;
    assert!(waits.is_err(), "request 22 waits for a free stream");
    let (late, ()) = tokio::join!(bed.open("/watch/pods"), async {
        sleep(SECOND).await;
        assert_eq!(bed.group.status().waiting, 1, "the wait shows in the state");
        streams.pop();
    });
    assert!(late.is_ok(), "a stream that ends gives its place to the request that waits");
    assert_eq!(bed.group.status().waiting, 0);

    // The extra connections close when no stream used them for some time.
    drop((streams, late));
    sleep(62 * SECOND).await;
    assert_eq!(bed.group.status().channels.len(), 1);
    assert_eq!(bed.status().state, ChannelState::Live, "the first connection stays");

    // A closed connection leaves its place: the next burst gets three connections again.
    let burst = futures_util::future::join_all((0..21).map(|_| bed.open("/watch/pods")));
    let burst = timeout(5 * SECOND, burst).await.expect("a free stream for each request");
    assert!(burst.iter().all(Result::is_ok));
    assert_eq!((bed.group.status().channels.len(), bed.server.dials()), (3, 5));
}

/// A burst at the start of a connection gets the budget for the start limit of hyper, 100
/// streams. The limit of the server can be higher, and a request that waits gets that room.
#[tokio::test(start_paused = true)]
async fn a_higher_limit_of_the_server_gives_room_to_a_request_that_waits() {
    let server = MemoryServer::new();
    server.set_stream_limit(150);
    let bed = Bed::on(server, ChannelSettings { max_connections: 1, ..settings() });
    let burst = futures_util::future::join_all((0..71).map(|_| bed.open("/watch/pods")));
    let burst = timeout(5 * SECOND, burst).await.expect("room for request 71 after the limit of the server came");
    assert!(burst.iter().all(Result::is_ok));
    let status = bed.status();
    assert_eq!((status.server_stream_limit, status.stream_budget, status.streams_open), (Some(150), 105, 71));
}

/// A caller that reads slowly, or not at all, fills the window of its stream. That must
/// not take the connection from the other callers: the state would stay live with no data.
#[tokio::test(start_paused = true)]
async fn bodies_that_nobody_reads_do_not_stop_the_connection() {
    let small =
        ChannelSettings { stream_window: 64 * KIB, connection_window: 256 * KIB, max_connections: 1, ..settings() };
    let mut bed = Bed::start(small);
    let live = bed.until_state(ChannelState::Live).await;
    assert_eq!(live.stream_budget, 3);

    // Each stream of the budget gets a body with no end, and nobody reads it.
    let mut unread = Vec::new();
    for _ in 0..live.stream_budget {
        unread.push(bed.open("/flood").await.expect("a stream"));
    }
    sleep(SECOND).await;
    assert!(timeout(SECOND, bed.open("/flood")).await.is_err(), "one more request waits for a free stream");

    // The test after a wake is one more stream. Its answer needs room in the connection window.
    bed.signal.raise(Cause::Wake);
    sleep(20 * SECOND).await;
    let status = bed.status();
    assert_eq!(
        (status.state, status.connections, status.last_error),
        (ChannelState::Live, 1, None),
        "the test of the connection and its ping got their answers"
    );

    // A reader that starts late finds its data.
    let (answer, _stream) = unread.pop().expect("a stream");
    let piece = answer.into_body().frame().await.expect("a frame").expect("the stream works");
    assert!(piece.into_data().is_ok_and(|data| !data.is_empty()));
}
