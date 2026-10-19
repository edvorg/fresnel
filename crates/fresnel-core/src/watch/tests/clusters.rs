//! The check of the cluster after a lost connection: its identity and its version. A new
//! cluster at the same address, or a restore of a backup, must give a list before the store
//! shows "live" again.

use tokio::time::sleep;

use super::bench::{Bench, LIVE, SECOND, is_pause, pairs, settings};
use crate::error::CoreError;
use crate::net::ErrorClass;
use crate::store::{StoreState, Wait};
use crate::watch::lanes::{Identity, LaneStates, Path};

/// No channel of the cluster is live.
const LOST: LaneStates = LaneStates { watches: Path::NoChannel, requests: Path::NoChannel, ..LIVE };

/// The channels are live again on new connections. The task of the identity did not read the
/// cluster behind them yet.
fn back(connects: u64) -> LaneStates {
    LaneStates { connects, ..LIVE }
}

fn is_checking(state: &StoreState) -> bool {
    *state == StoreState::Waiting(Wait::Cluster)
}

fn is_listing(state: &StoreState) -> bool {
    matches!(state, StoreState::Listing { .. })
}

/// After a lost connection, the store does not show "live" until the task of the identity
/// read the cluster behind the new connections and the version of the server is not older
/// than its cursor. It shows "checks the cluster" in that time and sends no request: a soak
/// saw a store "live" with the objects of an old cluster for 20 s while that read was on its
/// way, and the watch after the old cursor sent nothing.
#[tokio::test(start_paused = true)]
async fn after_a_lost_connection_the_store_is_live_only_after_the_check_of_the_cluster() {
    let (mut bench, _events) = Bench::live(settings()).await;
    bench.lanes.send_replace(LOST);
    bench.until_state("the lost channel", |state| *state == StoreState::Waiting(Wait::Channel)).await;
    bench.script.version_now("7");
    let _next = bench.script.watch_opens();
    bench.lanes.send_replace(back(1));
    let checking = bench.until_state("the check", is_checking).await;
    assert_eq!(checking.state.to_string(), "checks the cluster");
    sleep(30 * SECOND).await;
    let waits = bench.reader.read();
    assert_eq!((&waits.state, waits.since), (&checking.state, checking.since), "no other state while the read is out");
    assert_eq!(bench.script.take_calls(), Vec::<String>::new(), "no request before the identity");

    bench.lanes.send_modify(|lanes| lanes.identity.checked = 1);
    bench.until_state("the watch", StoreState::is_live).await;
    assert_eq!(bench.elapsed(), 30 * SECOND, "live at once after the check");
    assert_eq!(bench.script.take_calls(), ["list of 1", "watch after 7 for 60 s"]);
}

/// A changed cluster gives the store a list before it shows "live": the watch after the old
/// cursor never starts, so no view shows the old objects as current.
#[tokio::test(start_paused = true)]
async fn an_other_cluster_after_a_lost_connection_gives_a_list_before_the_store_is_live() {
    let (mut bench, _events) = Bench::live(settings()).await;
    bench.lanes.send_replace(LOST);
    bench.until_state("the lost channel", |state| *state == StoreState::Waiting(Wait::Channel)).await;
    bench.lanes.send_replace(back(1));
    bench.until_state("the check", is_checking).await;
    bench.lanes.send_modify(|lanes| lanes.identity = Identity { number: 1, checked: 1 });
    let listing = bench.until_state("the new list", is_listing).await;
    assert_eq!(listing.items.len(), 2, "the old objects stay with a visible state until the list ends");
    bench.script.page(&[("x", "2")], Some("3"), None);
    let _next = bench.script.watch_opens();
    bench.until_state("the watch of the new cluster", StoreState::is_live).await;
    assert_eq!(bench.objects(), pairs(&[("x", "2")]));
    assert_eq!(bench.script.take_calls(), ["list of 500", "watch after 3 for 60 s"], "no watch after the old cursor");
}

/// The renewal of a watch keeps the state "live". A renewal whose channel goes before its
/// answer would wait for the next connection in the connection layer and go on showing
/// "live", so the store drops it, and the next watch checks the cluster.
#[tokio::test(start_paused = true)]
async fn a_renewal_whose_channel_goes_before_its_answer_checks_the_cluster() {
    let (mut bench, events) = Bench::live(settings()).await;
    sleep(60 * SECOND).await;
    drop(events);
    sleep(SECOND).await;
    assert_eq!(bench.script.take_calls(), ["watch after 7 for 60 s"]);
    assert!(bench.reader.read().state.is_live(), "the renewal is on its way");
    bench.lanes.send_replace(LOST);
    bench.until_state("the lost channel", |state| *state == StoreState::Waiting(Wait::Channel)).await;
    bench.script.version_now("7");
    let _next = bench.script.watch_opens();
    bench.lanes.send_replace(back(1));
    bench.until_state("the check", is_checking).await;
    bench.lanes.send_modify(|lanes| lanes.identity.checked = 1);
    bench.until_state("the watch", StoreState::is_live).await;
    assert_eq!(bench.script.take_calls(), ["list of 1", "watch after 7 for 60 s"]);
}

/// The sequence of the soak: after a lost connection the check passed on the old cluster,
/// whose server then stopped, and the watch request failed. The next watch would reach a new
/// cluster at the same address. A request that failed for the network counts as a lost
/// connection, so that watch waits for the check of the cluster behind the new connections.
#[tokio::test(start_paused = true)]
async fn a_watch_request_that_failed_for_the_network_checks_the_cluster_again() {
    let (mut bench, _events) = Bench::live(settings()).await;
    bench.lanes.send_replace(LOST);
    bench.until_state("the lost channel", |state| *state == StoreState::Waiting(Wait::Channel)).await;
    bench.script.version_now("7");
    let reset = "watch channel: the connection ended before the answer: connection reset";
    bench.script.watch_fails(CoreError::new(ErrorClass::Network, reset));
    bench.lanes.send_replace(LaneStates { identity: Identity { number: 0, checked: 1 }, ..back(1) });
    bench.until_state("the pause", is_pause(1)).await;

    bench.lanes.send_replace(LaneStates { identity: Identity { number: 0, checked: 1 }, ..back(2) });
    bench.until_state("the check", is_checking).await;
    bench.lanes.send_modify(|lanes| lanes.identity = Identity { number: 1, checked: 2 });
    bench.script.page(&[("x", "2")], Some("3"), None);
    let _next = bench.script.watch_opens();
    bench.until_state("the watch of the new cluster", StoreState::is_live).await;
    assert_eq!(bench.objects(), pairs(&[("x", "2")]));
    let calls = ["list of 1", "watch after 7 for 60 s", "list of 500", "watch after 3 for 60 s"];
    assert_eq!(bench.script.take_calls(), calls);
}

/// A server that starts again with an older datastore, after a restore of a backup or as a
/// new cluster at the same address, takes a watch from a version that it does not have yet
/// and sends nothing. A fast soak found stores that showed "live" with the objects of the old
/// cluster for minutes after a recreate of k3s. The check after the lost connection finds the
/// older version of the server, and a new list follows at once. The identity of the cluster
/// comes first, and here the restored cluster has the same one.
#[tokio::test(start_paused = true)]
async fn a_cluster_that_went_back_to_an_older_version_gets_a_new_list_after_the_lost_connection() {
    let (mut bench, _events) = Bench::live(settings()).await;
    bench.lanes.send_replace(LaneStates { watches: Path::NoChannel, requests: Path::NoChannel, ..LIVE });
    bench.until_state("the lost channel", |state| *state == StoreState::Waiting(Wait::Channel)).await;
    bench.script.version_now("3");
    bench.lanes.send_replace(LIVE);
    let listing = bench.until_state("the new list", |state| matches!(state, StoreState::Listing { .. })).await;
    assert_eq!(listing.items.len(), 2, "the old objects stay until the list ends, with a visible state");
    bench.script.page(&[("x", "2")], Some("3"), None);
    let _next = bench.script.watch_opens();
    bench.until_state("the watch of the new cluster", StoreState::is_live).await;
    assert_eq!(bench.objects(), pairs(&[("x", "2")]));
    assert_eq!(bench.script.take_calls(), ["list of 1", "list of 500", "watch after 3 for 60 s"]);
}

/// A new cluster can pass the version of the store before the connection comes back: a soak
/// with two recreates in a row saw a store "live" with the old object of a name for minutes.
/// Its first object has the name of an object of the store and an other UID. That check
/// holds for a user who cannot read `kube-system`, whose identity check passes with no read.
#[tokio::test(start_paused = true)]
async fn a_new_cluster_past_the_version_of_the_store_gets_a_new_list_after_the_lost_connection() {
    let (mut bench, _events) = Bench::live(settings()).await;
    bench.lanes.send_replace(LaneStates { watches: Path::NoChannel, requests: Path::NoChannel, ..LIVE });
    bench.until_state("the lost channel", |state| *state == StoreState::Waiting(Wait::Channel)).await;
    bench.script.new_cluster_now("a", "40");
    bench.lanes.send_replace(LIVE);
    bench.until_state("the new list", |state| matches!(state, StoreState::Listing { .. })).await;
    bench.script.page(&[("a", "3"), ("c", "12")], Some("40"), None);
    let _next = bench.script.watch_opens();
    bench.until_state("the watch of the new cluster", StoreState::is_live).await;
    assert_eq!(bench.objects(), pairs(&[("a", "3"), ("c", "12")]));
    assert_eq!(bench.script.take_calls(), ["list of 1", "list of 500", "watch after 40 for 60 s"]);
}

/// A new cluster can also pass the check of one object: its first object can have no
/// namesake in the store. The cluster then gets a new number (`identity`), and the open watch
/// ends with a new list; so does the next watch of a store that waits in a pause.
#[tokio::test(start_paused = true)]
async fn an_other_cluster_ends_the_watch_and_the_next_one_with_a_new_list() {
    let (mut bench, _events) = Bench::live(settings()).await;
    bench.script.page(&[("x", "2")], Some("3"), None);
    let next = bench.script.watch_opens();
    bench.lanes.send_modify(|lanes| lanes.identity.number = 1);
    bench
        .until("the objects of the other cluster", |snapshot| snapshot.state.is_live() && snapshot.items.len() == 1)
        .await;
    assert_eq!(bench.script.take_calls(), ["list of 500", "watch after 3 for 60 s"]);

    next.send(Err(CoreError::new(ErrorClass::Network, "reset"))).expect("the watch is open");
    bench
        .until_state("the pause", |state| matches!(state, StoreState::Attempt { .. } | StoreState::Backoff { .. }))
        .await;
    bench.lanes.send_modify(|lanes| lanes.identity.number = 2);
    bench.script.page(&[("y", "1")], Some("2"), None);
    let _last = bench.script.watch_opens();
    bench.until_state("the watch of the third cluster", StoreState::is_live).await;
    assert_eq!(bench.objects(), pairs(&[("y", "1")]));
    assert_eq!(bench.script.take_calls(), ["list of 500", "watch after 2 for 60 s"], "no watch after the old cursor");
}
