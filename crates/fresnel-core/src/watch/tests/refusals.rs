//! The answers of a server that refuses: HTTP 403, HTTP 404 and HTTP 429.

use std::time::Duration;

use tokio::time::sleep;

use super::bench::{Bench, LIVE, SECOND, pairs, settings};
use crate::error::CoreError;
use crate::net::{Cause, ErrorClass};
use crate::store::StoreState;
use crate::watch::lanes::LaneStates;

fn forbidden() -> CoreError {
    CoreError::answered(403, "secrets is forbidden: User \"dev\" cannot list resource \"secrets\" (HTTP 403)")
}

/// The store asks again after some time, and the state does not show each try.
#[tokio::test(start_paused = true)]
async fn http_403_is_the_state_forbidden_and_the_store_asks_again_with_a_list() {
    let mut bench = Bench::start(settings());
    bench.script.list_fails(forbidden());
    let refused = bench.until_state("the refusal", |state| matches!(state, StoreState::Forbidden { .. })).await;
    assert_eq!(
        refused.state.to_string(),
        "forbidden: secrets is forbidden: User \"dev\" cannot list resource \"secrets\" (HTTP 403)"
    );
    let since = refused.since;

    bench.script.list_fails(forbidden());
    sleep(119 * SECOND).await;
    assert_eq!(bench.script.take_calls(), ["list of 500", "list of 500"], "one more list after 60 s");
    let still = bench.reader.read();
    assert_eq!((&still.state, still.since), (&refused.state, since), "no other state showed between the two lists");

    // The user got the permission. The store is live with no restart of the app.
    bench.script.page(&[("a", "5")], Some("7"), None);
    let events = bench.script.watch_opens();
    bench.until_state("the open watch", StoreState::is_live).await;
    assert_eq!(bench.elapsed(), 120 * SECOND);

    // The permission goes while the watch is open. The objects are old then, so the
    // next try is a list. "Retry now" starts it at once.
    events.send(Err(CoreError::new(ErrorClass::Network, "the stream ended"))).expect("the watch is open");
    bench.script.watch_fails(forbidden());
    let refused = bench.until_state("the refusal", |state| matches!(state, StoreState::Forbidden { .. })).await;
    assert_eq!(refused.items.len(), 1, "the objects stay");
    bench.script.take_calls();
    bench.signal.raise(Cause::RetryNow);
    bench.script.page(&[("a", "5"), ("b", "9")], Some("10"), None);
    bench.until_state("the start of the watch", |state| matches!(state, StoreState::Waiting(_))).await;
    assert_eq!(bench.script.take_calls(), ["list of 500", "watch after 10 for 60 s"]);
    assert_eq!(bench.objects(), pairs(&[("a", "5"), ("b", "9")]));
}

/// An older cluster does not have each kind of the app.
#[tokio::test(start_paused = true)]
async fn http_404_is_the_state_not_supported_with_a_new_list_each_10_minutes() {
    let mut bench = Bench::start(settings());
    let absent = || CoreError::answered(404, "the server could not find the requested resource (HTTP 404)");
    bench.script.list_fails(absent());
    bench.script.list_fails(absent());
    let refused = bench.until_state("the refusal", |state| matches!(state, StoreState::NotSupported { .. })).await;
    assert_eq!(refused.state.to_string(), "not supported: the server could not find the requested resource (HTTP 404)");
    sleep(1199 * SECOND).await;
    assert_eq!(bench.script.take_calls(), ["list of 500", "list of 500"]);
    sleep(2 * SECOND).await;
    assert_eq!(bench.script.take_calls(), ["list of 500"]);
    assert_eq!(bench.reader.read().state, refused.state);
}

/// The connection layer holds a request for the time of `Retry-After` and sends it again.
/// The store of that request shows the throttle and not a list with no progress.
#[tokio::test(start_paused = true)]
async fn a_request_that_waits_for_the_end_of_a_throttle_shows_that_state() {
    let mut bench = Bench::start(settings());
    bench.until_state("the list request", |state| *state == StoreState::Listing { loaded: 0 }).await;
    let until = bench.clock.now() + 5 * SECOND;
    bench.lanes.send_replace(LaneStates { throttled_until: Some(until), ..LIVE });
    bench.until_state("the throttle", |state| *state == StoreState::Throttled { until }).await;
    sleep(5 * SECOND).await;
    bench.script.page(&[("a", "5")], Some("7"), None);
    bench.until_state("the answer", |state| matches!(state, StoreState::Waiting(_))).await;
    assert_eq!(bench.script.take_calls(), ["list of 500", "watch after 7 for 60 s"], "the request went out one time");
}

/// Plan section 8.11: the app obeys `Retry-After` and shows "throttled until".
#[tokio::test(start_paused = true)]
async fn a_throttle_of_the_server_is_a_state_with_its_time_and_no_signal_ends_it() {
    let mut bench = Bench::start(settings());
    let until = bench.clock.now() + 20 * SECOND;
    // The connection layer gives the time of `Retry-After` and fails each request before it.
    bench.lanes.send_replace(LaneStates { throttled_until: Some(until), ..LIVE });
    bench.script.page(&[("a", "5")], Some("7"), None);
    let throttled = bench.until_state("the throttle", |state| matches!(state, StoreState::Throttled { .. })).await;
    assert_eq!(throttled.state, StoreState::Throttled { until });
    sleep(5 * SECOND).await;
    bench.signal.raise(Cause::RetryNow);
    sleep(14 * SECOND).await;
    assert_eq!(bench.script.take_calls(), Vec::<String>::new(), "no request before the time of the server");

    // The watch gets HTTP 429 with no time in the connection layer: one second.
    bench.script.watch_fails(CoreError::answered(429, "Too many requests (HTTP 429)"));
    let throttled = bench
        .until_state("the second throttle", |state| *state == StoreState::Throttled { until: until + SECOND })
        .await;
    assert_eq!(bench.elapsed(), 20 * SECOND);
    assert_eq!(throttled.items.len(), 1);
    assert_eq!(bench.script.take_calls(), ["list of 500", "watch after 7 for 60 s"]);

    // The throttle starts while the next request is on its way. The connection layer does
    // not send a request whose time limit ends before the throttle: its error has no status.
    bench.until_state("the next request", |state| matches!(state, StoreState::Waiting(_))).await;
    let later = until + 26 * SECOND;
    bench.lanes.send_modify(|lanes| lanes.throttled_until = Some(later));
    bench
        .script
        .watch_fails(CoreError::new(ErrorClass::Server, "the server throttles requests for 25s more (HTTP 429)"));
    bench.until_state("the third throttle", |state| *state == StoreState::Throttled { until: later }).await;
    assert_eq!(bench.elapsed(), Duration::from_secs(21));
    let _events = bench.script.watch_opens();
    bench.until_state("the open watch", StoreState::is_live).await;
    assert_eq!(bench.elapsed(), 46 * SECOND);
}
