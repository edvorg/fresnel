//! What follows a failure: the pause, its early end, HTTP 410 and a fault of the task.

use tokio::time::sleep;

use super::bench::{Bench, LIVE, MS, SECOND, error_event, event, is_pause, pairs, settings};
use crate::error::CoreError;
use crate::net::task::Backoff;
use crate::net::{Cause, ErrorClass};
use crate::store::StoreState;
use crate::watch::WatchSettings;
use crate::watch::lanes::{LaneStates, Path};

fn broke() -> CoreError {
    CoreError::new(ErrorClass::Network, "the connection broke")
}

/// Plan section 9.1. The loop of the prototype counted each reset, also after a watch
/// that worked: after 7 resets in one minute it waited 17 s.
#[tokio::test(start_paused = true)]
async fn the_count_of_failures_starts_again_at_the_first_answer() {
    let mut bench = Bench::start(settings());
    for (failures, at) in [(1, 0), (2, 100), (3, 300)] {
        bench.script.list_fails(broke());
        bench.until_state("the pause", is_pause(failures)).await;
        assert_eq!(bench.elapsed(), at * MS, "the pause grows with each failure in sequence");
    }
    // The answer to the list ends the sequence. The next failure is the first one again. A
    // watch that failed for the network gets a check of the cluster before the next one.
    bench.script.page(&[("a", "5")], Some("7"), None);
    bench.script.watch_fails(broke());
    bench.until_state("the pause", is_pause(1)).await;
    assert_eq!(bench.elapsed(), 700 * MS);
    bench.script.version_now("7");

    // Eight resets of a watch that works. The channel counts the loss of a connection, so
    // the store takes no pause of its own: the next watch starts at once (plan section 8.13),
    // after a check that the server did not go back.
    for reset in 1..=8 {
        let events = bench.script.watch_opens();
        bench.until_state("the open watch", StoreState::is_live).await;
        sleep(5 * SECOND).await;
        bench.script.version_now("7");
        events.send(Err(broke())).expect("the watch is open");
        let attempt = bench.until_state("the next watch", |state| matches!(state, StoreState::Attempt { .. })).await;
        assert_eq!(attempt.state.to_string(), "attempt after 1 failures: the connection broke");
        assert_eq!(bench.elapsed(), 800 * MS + reset * 5 * SECOND, "reset {reset}");
    }
    // A second failure with no answer between the two gets a pause.
    bench.script.watch_fails(broke());
    bench.until_state("the pause", is_pause(2)).await;
    let lists = bench.script.take_calls().iter().filter(|call| *call == "list of 500").count();
    assert_eq!(lists, 4, "no new list");
}

/// One signal of the cluster ends each retry delay: a wake, a network change or "retry
/// now". A connection that returns does the same. In the prototype, the last store was
/// live 26 s after the network returned.
#[tokio::test(start_paused = true)]
async fn the_signal_of_the_cluster_and_a_connection_that_returns_end_a_pause() {
    let slow = Backoff { first: 60 * SECOND, max: 60 * SECOND, jitter_percent: 0, reset_after: SECOND };
    let mut bench = Bench::start(WatchSettings { backoff: slow, ..settings() });
    bench.script.list_fails(broke());
    bench.until_state("the pause", is_pause(1)).await;
    sleep(2 * SECOND).await;
    bench.signal.raise(Cause::Wake);
    bench.script.list_fails(broke());
    bench.until_state("the pause", is_pause(2)).await;
    assert_eq!(bench.elapsed(), 2 * SECOND, "the signal ended a pause of 60 s");

    // The channel is lost during the pause and returns: the pause ends then.
    sleep(3 * SECOND).await;
    bench.lanes.send_replace(LaneStates { requests: Path::NoChannel, watches: Path::NoChannel, ..LIVE });
    sleep(4 * SECOND).await;
    bench.script.list_fails(broke());
    bench.lanes.send_replace(LIVE);
    bench.until_state("the pause", is_pause(3)).await;
    assert_eq!(bench.elapsed(), 9 * SECOND);

    // With no signal, the pause has its full length.
    bench.script.list_fails(broke());
    bench.until_state("the pause", is_pause(4)).await;
    assert_eq!(bench.elapsed(), 69 * SECOND);
}

/// Plan section 9.1: a new list after HTTP 410, with a delay after a second one. A server
/// that gives a list and then HTTP 410 to each watch must not get a list with no pause.
#[tokio::test(start_paused = true)]
async fn http_410_gives_a_new_list_at_once_and_a_second_one_gives_a_pause() {
    let (mut bench, events) = Bench::live(settings()).await;
    sleep(10 * SECOND).await;
    let expired = || error_event(410, "Expired", "too old resource version: 7 (9000)");
    events.send(Ok(expired())).expect("the watch is open");
    bench.script.page(&[("a", "5"), ("c", "90")], Some("100"), None);
    // The server refuses the watch after the new list with the status HTTP 410.
    bench.script.watch_fails(CoreError::answered(410, "too old resource version: 100 (9001) (HTTP 410)"));
    let paused = bench.until_state("the pause", is_pause(1)).await;
    assert_eq!(bench.elapsed(), 10 * SECOND, "the first HTTP 410 costs no pause");
    assert_eq!(bench.script.take_calls(), ["list of 500", "watch after 100 for 60 s"]);
    assert_eq!(bench.objects(), pairs(&[("a", "5"), ("c", "90")]), "the new list replaced the objects");
    assert_eq!(paused.state.to_string(), "pause after 1 failures: too old resource version: 100 (9001) (HTTP 410)");

    // After the pause comes a list, not a watch: the version of the store is gone.
    bench.script.page(&[("a", "5"), ("c", "90")], Some("101"), None);
    let events = bench.script.watch_opens();
    let live = bench.until_state("the open watch", StoreState::is_live).await;
    assert_eq!(bench.elapsed(), 10 * SECOND + 100 * MS);
    assert_eq!(live.generation, 2, "a list with no change is not a change for a reader");
    assert_eq!(bench.script.take_calls(), ["list of 500", "watch after 101 for 60 s"]);

    // A watch that worked ends the sequence: the next HTTP 410 gets its list at once.
    events.send(Ok(event("ADDED", "d", "102"))).expect("the watch is open");
    events.send(Ok(expired())).expect("the watch is open");
    bench.script.page(&[("d", "102")], Some("103"), None);
    bench.until("the new list", |snapshot| snapshot.items.len() == 1).await;
    assert_eq!(bench.elapsed(), 10 * SECOND + 100 * MS);
}

/// A page token is for one version of the data. When that version is gone, the list
/// starts again, and a second HTTP 410 gets a pause.
#[tokio::test(start_paused = true)]
async fn http_410_for_the_token_of_a_list_page_starts_the_list_again() {
    let mut bench = Bench::start(settings());
    let gone = || CoreError::answered(410, "the provided continue parameter is too old (HTTP 410)");
    // A good first page ends no sequence of HTTP 410: the pauses are 0, 100 and 200 ms.
    for (failures, at) in [(1, 0), (2, 100)] {
        for _ in 0..if failures == 1 { 2 } else { 1 } {
            bench.script.page(&[("a", "5")], None, Some("t1"));
            bench.script.list_fails(gone());
        }
        bench.until_state("the pause", is_pause(failures)).await;
        assert_eq!(bench.elapsed(), at * MS);
    }
    bench.script.page(&[("a", "5")], Some("7"), None);
    bench.until_state("the good list", |state| matches!(state, StoreState::Waiting(_))).await;
    assert_eq!(bench.elapsed(), 300 * MS);
    let lists = ["list of 500", "list after t1"].repeat(3);
    assert_eq!(bench.script.take_calls(), [lists, vec!["list of 500", "watch after 7 for 60 s"]].concat());
}

/// Design rule 1: a task that panics gets a state, a journal entry and a restart. In the
/// prototype, a store whose task ended showed "live" with objects that did not change.
#[tokio::test(start_paused = true)]
async fn a_panic_of_the_task_of_a_store_gives_a_state_a_report_and_a_restart() {
    let (mut bench, events) = Bench::live(settings()).await;
    sleep(90 * SECOND).await;
    bench.script.version_now("7");
    bench.script.watch_panics("index 4 of 3");
    drop(events);
    let failed = bench.until_state("the fault", |state| matches!(state, StoreState::InternalError { .. })).await;
    assert_eq!(failed.state.to_string(), "internal error: the task of the store panicked: index 4 of 3");
    assert_eq!((failed.items.len(), failed.since), (2, bench.clock.now()), "the objects stay");
    assert_eq!(bench.reports(), ["task store [bench/Secret] panicked: index 4 of 3; restart in 500 ms"]);

    // The new run of the task starts with a list: the run before can have lost an event.
    bench.script.list_panics("no luck");
    bench.script.page(&[("a", "5"), ("b", "8")], Some("9"), None);
    let _events = bench.script.watch_opens();
    let live = bench.until_state("the open watch", StoreState::is_live).await;
    assert_eq!(bench.elapsed(), 91 * SECOND, "two restarts with a pause of 500 ms");
    assert_eq!((bench.objects(), live.generation), (pairs(&[("a", "5"), ("b", "8")]), 2));
    assert_eq!(bench.reports().len(), 2);
    assert_eq!(
        bench.script.take_calls(),
        ["list of 1", "watch after 7 for 60 s", "list of 500", "list of 500", "watch after 9 for 60 s"],
        "the stalled watch at 70 s, the check of its version, the next watch"
    );
}

/// The task that follows the connections of the cluster is gone: the loop cannot know
/// the state of its channel, so its run ends with a fault.
#[tokio::test(start_paused = true)]
async fn a_loop_with_no_source_of_the_channel_state_ends_with_a_fault() {
    let mut bench = Bench::start(settings());
    bench.lanes.send_replace(LaneStates { requests: Path::NoChannel, ..LIVE });
    bench.until_state("the wait", |state| matches!(state, StoreState::Waiting(_))).await;
    let (other, _) = tokio::sync::watch::channel(LIVE);
    drop(std::mem::replace(&mut bench.lanes, other));
    let failed = bench.until_state("the fault", |state| matches!(state, StoreState::InternalError { .. })).await;
    assert_eq!(
        failed.state.to_string(),
        "internal error: the task that follows the connections of the cluster is gone"
    );
    assert_eq!(bench.reports().len(), 1);
}
