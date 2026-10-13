//! The list of the loop: pages, the list slot and the step to the watch.

use std::sync::Arc;
use std::time::Duration;

use futures_util::FutureExt as _;

use super::bench::{Bench, MS, SECOND, pairs, settings};
use crate::error::CoreError;
use crate::kinds::Kind;
use crate::net::ErrorClass;
use crate::store::{StoreState, Wait};

#[tokio::test(start_paused = true)]
async fn the_list_reads_each_page_and_the_watch_starts_after_the_version_of_the_list() {
    let mut bench = Bench::start(settings());
    bench.script.page(&[("a", "5"), ("b", "6")], None, Some("t1"));
    let first_page = bench.until_state("the first page", |state| *state == StoreState::Listing { loaded: 2 }).await;
    assert_eq!((first_page.items.len(), first_page.generation), (0, 0), "a list shows when it is complete");
    assert_eq!(first_page.last_contact, Some(bench.clock.now()), "a page is data of the server");

    bench.script.page(&[("c", "4")], Some("7"), None);
    let _events = bench.script.watch_opens();
    let live = bench.until_state("the open watch", StoreState::is_live).await;
    assert_eq!((live.generation, live.since), (1, bench.clock.now()));
    assert_eq!(bench.objects(), pairs(&[("a", "5"), ("b", "6"), ("c", "4")]));
    assert_eq!(bench.script.take_calls(), ["list of 500", "list after t1", "watch after 7 for 60 s"]);
    assert_eq!(bench.elapsed(), Duration::ZERO, "no step of a good start waits");
}

#[tokio::test(start_paused = true)]
async fn a_list_waits_for_a_slot_of_its_cluster_and_shows_it() {
    let mut bench = Bench::start(settings());
    let gate = Arc::clone(&bench.gate);
    let mut slots = Vec::new();
    for _ in 0..4 {
        slots.push(gate.take(Kind::Pod).await);
    }
    bench.script.page(&[("a", "5")], Some("7"), None);
    bench.until_state("the wait for a slot", |state| *state == StoreState::Waiting(Wait::ListSlot)).await;
    tokio::time::sleep(30 * SECOND).await;
    assert_eq!(bench.script.take_calls(), Vec::<String>::new(), "four lists of the cluster run");

    slots.pop();
    bench.until_state("the list", |state| *state == StoreState::Waiting(Wait::Stream)).await;
    assert_eq!(bench.script.take_calls(), ["list of 500", "watch after 7 for 60 s"]);
    assert!(gate.take(Kind::Pod).now_or_never().is_some(), "the list gave its slot back");
}

#[tokio::test(start_paused = true)]
async fn a_list_that_fails_starts_again_at_its_first_page_after_a_pause() {
    let mut bench = Bench::start(settings());
    bench.script.page(&[("a", "5")], None, Some("t1"));
    bench.script.list_fails(CoreError::new(ErrorClass::Network, "the connection broke"));
    let paused = bench.until_state("the pause", |state| matches!(state, StoreState::Backoff { .. })).await;
    let StoreState::Backoff { failures, reason, until } = &paused.state else { panic!("{:?}", paused.state) };
    assert_eq!((*failures, reason.to_string().as_str()), (1, "the connection broke"));
    assert_eq!(*until, bench.clock.now() + 100 * MS);
    assert_eq!(paused.items.len(), 0, "half of a list is not a list");

    // The next request is in progress, and the state tells that and not a time in the past.
    let attempt = bench.until_state("the attempt", |state| matches!(state, StoreState::Attempt { .. })).await;
    assert_eq!(attempt.state.to_string(), "attempt after 1 failures: the connection broke");
    assert_eq!(bench.elapsed(), 100 * MS);
    assert_eq!(bench.script.take_calls(), ["list of 500", "list after t1", "list of 500"]);
}

/// With no version, the watch would start at "now" and lose a change with no error.
#[tokio::test(start_paused = true)]
async fn a_list_with_no_resource_version_or_with_an_object_with_no_name_is_a_failure() {
    let mut bench = Bench::start(settings());
    bench.script.page(&[("a", "5")], None, None);
    let paused = bench.until_state("the pause", |state| matches!(state, StoreState::Backoff { .. })).await;
    assert_eq!(paused.state.to_string(), "pause after 1 failures: the list of the server has no resource version");

    bench.script.page_with_no_name();
    let paused = bench.until_state("the pause", |state| matches!(state, StoreState::Backoff { failures: 2, .. })).await;
    assert_eq!(paused.state.to_string(), "pause after 2 failures: an object in the answer of the server has no name");
    assert_eq!(bench.objects(), pairs(&[]));
    assert_eq!(bench.script.take_calls(), ["list of 500", "list of 500"], "no watch starts with no good list");
}

/// Bad data of a server: a page that names the token of its own request as the next one.
/// Such a list has no end, and it would ask the server again with no pause.
#[tokio::test(start_paused = true)]
async fn a_list_page_with_the_token_of_its_own_request_is_a_failure() {
    let mut bench = Bench::start(settings());
    bench.script.page(&[("a", "5")], None, Some("t1"));
    bench.script.page(&[("b", "6")], None, Some("t1"));
    let paused = bench.until_state("the pause", |state| matches!(state, StoreState::Backoff { .. })).await;
    let expected = "pause after 1 failures: the list of the server gave the token of a page two times";
    assert_eq!((paused.state.to_string().as_str(), paused.items.len()), (expected, 0));
    assert_eq!(bench.script.take_calls(), ["list of 500", "list after t1"]);
}
