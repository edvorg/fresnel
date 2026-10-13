//! One watch of the loop: events, batches, the stall limit and the state of the channel.

use std::time::Duration;

use tokio::task::yield_now;
use tokio::time::sleep;

use super::bench::{Bench, LIVE, MS, SECOND, error_event, event, pairs, settings};
use crate::error::CoreError;
use crate::net::ErrorClass;
use crate::store::{StoreState, Wait};
use crate::watch::lanes::{LaneStates, Path};

#[tokio::test(start_paused = true)]
async fn the_next_watch_starts_after_the_last_event_or_bookmark_of_the_one_before() {
    let (mut bench, events) = Bench::live(settings()).await;
    sleep(40 * SECOND).await;
    for (kind, name, version) in [("ADDED", "c", "8"), ("MODIFIED", "a", "9"), ("DELETED", "b", "10")] {
        events.send(Ok(event(kind, name, version))).expect("the watch is open");
    }
    let changed = bench.until("the three events", |snapshot| snapshot.generation == 3).await;
    assert_eq!(bench.objects(), pairs(&[("a", "9"), ("c", "8")]));
    assert_eq!(changed.last_contact, Some(bench.clock.now() - 30 * MS), "the time of the events, not of the batch");

    // A bookmark changes no object. It moves the cursor and tells that the watch works.
    sleep(15 * SECOND).await;
    let bookmark =
        r#"{"type":"BOOKMARK","object":{"kind":"Secret","apiVersion":"v1","metadata":{"resourceVersion":"15"}}}"#;
    events.send(Ok(serde_json::from_str(bookmark).expect("a bookmark"))).expect("the watch is open");
    let now = bench.clock.now();
    let marked = bench.until("the bookmark", |snapshot| snapshot.last_contact == Some(now)).await;
    assert_eq!((marked.generation, changed.generation), (3, 3), "a bookmark is not a change of the objects");

    // The server ends each watch at its timeout. The next one starts at once.
    sleep(5 * SECOND).await;
    drop(events);
    let next = bench.script.watch_opens();
    next.send(Ok(event("ADDED", "d", "16"))).expect("the watch is open");
    bench.until("an event of the next watch", |snapshot| snapshot.items.len() == 3).await;
    assert_eq!(bench.script.take_calls(), ["watch after 15 for 60 s"]);
    assert_eq!(bench.elapsed(), 60 * SECOND + 30 * MS, "no pause before the next watch");
}

/// Design rule 4: a view marks each row from a store that is not live. The server ends
/// each watch after 60 to 90 s. A store with a state for each such end would get that
/// mark each minute, with no fault.
#[tokio::test(start_paused = true)]
async fn a_store_stays_live_while_the_renewal_of_its_watch_starts() {
    let (mut bench, events) = Bench::live(settings()).await;
    let live_since = bench.reader.read().since;
    sleep(60 * SECOND).await;
    events.send(Ok(event("ADDED", "c", "8"))).expect("the watch is open");
    bench.until("the first change", |snapshot| snapshot.items.len() == 3).await;
    // A change that waits for its batch shows at the end of the watch, because the answer
    // to the next request can need some seconds.
    sleep(5 * MS).await;
    events.send(Ok(event("ADDED", "d", "9"))).expect("the watch is open");
    drop(events);
    bench.until("the change before the end", |snapshot| snapshot.items.len() == 4).await;
    assert_eq!(bench.elapsed(), 60 * SECOND + 5 * MS);
    sleep(3 * SECOND).await;
    assert_eq!(bench.script.take_calls(), ["watch after 9 for 60 s"]);
    let renewing = bench.reader.read();
    assert_eq!((&renewing.state, renewing.since), (&StoreState::Live, live_since), "no state between two watches");

    let events = bench.script.watch_opens();
    events.send(Ok(event("ADDED", "e", "10"))).expect("the watch is open");
    let renewed = bench.until("an event of the new watch", |snapshot| snapshot.items.len() == 5).await;
    assert_eq!((&renewed.state, renewed.since), (&StoreState::Live, live_since));

    // Only the start after such an end is a renewal. A failure of the new watch shows.
    events.send(Err(CoreError::new(ErrorClass::Network, "the connection broke"))).expect("the watch is open");
    let attempt = bench.until_state("the next watch", |state| matches!(state, StoreState::Attempt { .. })).await;
    assert_eq!(attempt.items.len(), 5, "the objects stay");

    // A renewal that has to wait is a wait like each other one: the store shows each step.
    let events = bench.script.watch_opens();
    bench.until_state("the open watch", StoreState::is_live).await;
    sleep(60 * SECOND).await;
    bench.lanes.send_replace(LaneStates { credential_failed: true, ..LIVE });
    drop(events);
    bench.until_state("the wait for a token", |state| *state == StoreState::Waiting(Wait::Credential)).await;
    bench.lanes.send_replace(LIVE);
    bench.until_state("the start of the watch", |state| *state == StoreState::Waiting(Wait::Stream)).await;
}

/// Plan section 9.2. At 200 changes in one second, the readers get 30 snapshots and not 200.
#[tokio::test(start_paused = true)]
async fn the_changes_of_30_ms_reach_the_readers_as_one_snapshot() {
    let (mut bench, events) = Bench::live(settings()).await;
    sleep(SECOND).await;
    events.send(Ok(event("ADDED", "c", "8"))).expect("the watch is open");
    let first = bench.until("the first change", |snapshot| snapshot.generation == 2).await;
    assert_eq!((first.items.len(), bench.elapsed()), (3, SECOND), "a change after a quiet time shows at once");

    for (after, name) in [(5, "d"), (5, "e"), (5, "f")] {
        sleep(after * MS).await;
        events.send(Ok(event("ADDED", name, "9"))).expect("the watch is open");
        yield_now().await;
    }
    assert_eq!(bench.reader.read().items.len(), 3, "the changes wait for the end of the batch");
    let batch = bench.until("the batch", |snapshot| snapshot.generation > 2).await;
    assert_eq!((batch.generation, batch.items.len()), (3, 6), "three events are one change for a reader");
    assert_eq!(bench.elapsed(), SECOND + 30 * MS);

    // An event that changes nothing is not a new generation: no view computes its rows again.
    sleep(SECOND).await;
    events.send(Ok(event("MODIFIED", "f", "9"))).expect("the watch is open");
    let now = bench.clock.now();
    let same = bench.until("the event", |snapshot| snapshot.last_contact == Some(now)).await;
    assert_eq!(same.generation, 3);
}

/// A change that waits for its batch must not wait for the next request or for a pause.
#[tokio::test(start_paused = true)]
async fn a_change_that_waits_for_its_batch_shows_at_the_end_of_the_watch() {
    let (mut bench, events) = Bench::live(settings()).await;
    events.send(Ok(event("ADDED", "c", "8"))).expect("the watch is open");
    events.send(Ok(event("ADDED", "d", "9"))).expect("the watch is open");
    events.send(Ok(error_event(500, "InternalError", "etcdserver: no leader"))).expect("the watch is open");
    let failed = bench.until_state("the next watch", |state| matches!(state, StoreState::Attempt { .. })).await;
    assert_eq!((failed.items.len(), failed.generation), (4, 2), "the objects before the error, as one change");
    assert_eq!(failed.state.to_string(), "attempt after 1 failures: etcdserver: no leader (InternalError, HTTP 500)");
    assert_eq!(bench.elapsed(), Duration::ZERO);
    assert_eq!(bench.script.take_calls(), ["watch after 9 for 60 s"]);
}

/// Plan section 9.1: the stall limit is the server timeout plus 10 s.
#[tokio::test(start_paused = true)]
async fn a_watch_with_no_event_and_no_bookmark_in_the_stall_limit_starts_again() {
    let (mut bench, events) = Bench::live(settings()).await;
    sleep(30 * SECOND).await;
    events.send(Ok(event("ADDED", "c", "8"))).expect("the watch is open");
    let stalled = bench.until_state("the stall", |state| matches!(state, StoreState::Attempt { .. })).await;
    assert_eq!(bench.elapsed(), 100 * SECOND, "70 s after the last event");
    assert_eq!(stalled.state.to_string(), "attempt after 1 failures: the watch gave no event and no bookmark in 70 s");
    assert!(events.is_closed(), "the stream of the stalled watch is closed");
    assert_eq!(stalled.items.len(), 3, "the objects stay");

    let _next = bench.script.watch_opens();
    bench.until_state("the next watch", StoreState::is_live).await;
    assert_eq!(bench.script.take_calls(), ["watch after 8 for 60 s"], "no list: the watch continues");
    assert_eq!(bench.elapsed(), 100 * SECOND, "the new watch started at once");
}

/// The server answers each watch with a good status and ends it at once. Each such end
/// is a failure, so the pause grows.
#[tokio::test(start_paused = true)]
async fn a_server_that_ends_each_watch_at_once_gets_a_pause_that_grows() {
    let mut bench = Bench::start(settings());
    bench.script.page(&[("a", "5")], Some("7"), None);
    for expected in [(1, 0), (2, 100), (3, 300)] {
        drop(bench.script.watch_opens());
        let paused = bench
            .until_state(
                "the pause",
                |state| matches!(state, StoreState::Backoff { failures, .. } if *failures == expected.0),
            )
            .await;
        assert_eq!(bench.elapsed(), expected.1 * MS);
        assert_eq!(
            paused.state.to_string(),
            format!("pause after {} failures: the server ended the watch after 0 ms with no event", expected.0)
        );
    }
    // A watch that stays open for one second did work, and the sequence of failures ends.
    let events = bench.script.watch_opens();
    bench.until_state("the open watch", StoreState::is_live).await;
    sleep(SECOND).await;
    drop(events);
    drop(bench.script.watch_opens());
    bench.until_state("the pause", |state| matches!(state, StoreState::Backoff { failures: 1, .. })).await;
}

/// The store follows the state of its channel: with no live channel for watches, the
/// store is not live, also when its stream gave no error yet. A silent connection thus
/// shows as soon as the channel finds it.
#[tokio::test(start_paused = true)]
async fn a_store_is_not_live_while_no_channel_for_watches_is_live() {
    let (mut bench, events) = Bench::live(settings()).await;
    sleep(20 * SECOND).await;
    bench.lanes.send_replace(LaneStates { watches: Path::NoChannel, ..LIVE });
    let waiting = bench.until_state("the lost channel", |state| *state == StoreState::Waiting(Wait::Channel)).await;
    assert_eq!((waiting.items.len(), waiting.since), (2, bench.clock.now()), "the objects stay, with the time");
    assert!(events.is_closed(), "the stream on the lost connection is closed");

    // No request goes into a connection that is gone, and nothing counts as a failure.
    sleep(600 * SECOND).await;
    assert_eq!(bench.script.take_calls(), Vec::<String>::new());
    let _next = bench.script.watch_opens();
    bench.lanes.send_replace(LIVE);
    bench.until_state("the watch on the new connection", StoreState::is_live).await;
    assert_eq!(bench.script.take_calls(), ["watch after 7 for 60 s"], "at once, and with no list");
    assert_eq!(bench.elapsed(), 620 * SECOND);
}

#[tokio::test(start_paused = true)]
async fn a_store_waits_for_a_token_and_for_a_channel_before_its_first_request() {
    let mut bench = Bench::start(settings());
    bench.lanes.send_replace(LaneStates { requests: Path::NoToken, ..LIVE });
    bench.script.page(&[("a", "5")], Some("7"), None);
    bench.until_state("the wait for a token", |state| *state == StoreState::Waiting(Wait::Credential)).await;
    bench.lanes.send_replace(LaneStates { requests: Path::NoChannel, ..LIVE });
    bench.until_state("the wait for a channel", |state| *state == StoreState::Waiting(Wait::Channel)).await;
    // The plugin failed: a request would wait for it and fail.
    bench.lanes.send_replace(LaneStates { credential_failed: true, ..LIVE });
    bench.until_state("the wait for a token", |state| *state == StoreState::Waiting(Wait::Credential)).await;
    sleep(60 * SECOND).await;
    assert_eq!(bench.script.take_calls(), Vec::<String>::new());

    bench.lanes.send_replace(LIVE);
    bench.until_state("the start of the watch", |state| *state == StoreState::Waiting(Wait::Stream)).await;
    assert_eq!(bench.script.take_calls(), ["list of 500", "watch after 7 for 60 s"]);
}
