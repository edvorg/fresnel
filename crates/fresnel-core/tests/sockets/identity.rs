//! The identity of the cluster behind one address: the UID of its Namespace kube-system,
//! which the fake server gives, after a lost connection.

use std::time::{Duration, SystemTime};

use fresnel_core::kinds::Kind;
use fresnel_core::net::NetSettings;
use fresnel_core::store::{StoreSettings, StoreState, StoreStatus, Wait};
use fresnel_testkit::{FakeApiServer, FakeCounters, Fault};
use tokio::time::sleep;

use crate::bed::ProxyBed;
use crate::common::{LIMIT, in_time};
use crate::store_bed::Stores;
use crate::store_faults::{RECOVERY, lists_and_watches, live_again_after};
use crate::stores::{KUBE_SYSTEM, STORES, is_version_check, of_path};

/// The time in which the test looks for a store that shows "live" or sends a request while
/// the read of the identity gets no answer.
const HELD: Duration = Duration::from_secs(2);

/// The Namespace kube-system with this UID, as the server gives it.
fn body(uid: &str) -> String {
    format!(r#"{{"metadata":{{"name":"kube-system","uid":"{uid}","resourceVersion":"4"}}}}"#)
}

/// Waits until the server got `count` reads of kube-system.
async fn reads(server: &FakeApiServer, count: usize) -> FakeCounters {
    server.wait_for(move |_| of_path(&server.requests(), KUBE_SYSTEM).len() >= count).await
}

/// The lists, the watches and the checks of the version of each kind that the server got.
fn requests(server: &FakeApiServer) -> Vec<(usize, usize, usize)> {
    let requests = server.requests();
    let of_kind = |kind: &Kind| {
        let path = kind.info().path();
        let (lists, watches) = lists_and_watches(server, &path);
        (lists, watches, of_path(&requests, &path).into_iter().filter(|seen| is_version_check(seen)).count())
    };
    Kind::ALL.iter().map(of_kind).collect()
}

/// The identity of the cluster changes behind one address, and its read gets no answer for
/// a time. No store shows "live" in that time: each one shows "checks the cluster" and sends
/// no request. Then each store lists the new cluster before it is live. A soak saw stores
/// "live" with the objects of the old cluster for 20 s while that read was on its way.
#[tokio::test]
async fn no_store_is_live_while_the_read_of_the_identity_after_a_lost_connection_is_on_its_way() {
    let bed = ProxyBed::start("held-identity", NetSettings::default()).await;
    bed.server.set_body(KUBE_SYSTEM, &body("first"));
    let stores = Stores::start(&bed.tasks, &bed.link, StoreSettings::default());
    stores.until_live(LIMIT).await;
    in_time("the read at the connect", reads(&bed.server, 1)).await;
    let before = requests(&bed.server);

    bed.server.set_body(KUBE_SYSTEM, &body("second"));
    bed.server.add_fault(Fault::no_answer().on_path(KUBE_SYSTEM));
    let lost = std::time::Instant::now();
    in_time("a reset", bed.proxy.reset_all()).await;
    let checking = |all: &[StoreStatus]| all.iter().all(|store| store.state == StoreState::Waiting(Wait::Cluster));
    stores.until("each store checks the cluster", RECOVERY, checking).await;
    println!("each store checked the cluster {:?} after the reset", lost.elapsed());
    sleep(HELD).await;
    let held = stores.stores.statuses();
    assert!(held.iter().all(|store| store.state == StoreState::Waiting(Wait::Cluster)), "{:#?}", stores.states());
    assert_eq!(requests(&bed.server), before, "no list, no watch and no check of the version before the identity");

    // The read that gets no answer ends with its connection, and the next one gives the UID.
    bed.server.clear_faults();
    let back = SystemTime::now();
    in_time("a reset", bed.proxy.reset_all()).await;
    let twice = |now: &[(usize, usize, usize)]| {
        now.iter().zip(&before).all(|(now, before)| *now == (before.0 * 2, before.1 * 2, before.2))
    };
    let live = |all: &[StoreStatus]| all.iter().all(|store| store.state.is_live() && store.since >= back);
    stores.until("each store is live after a list", RECOVERY, |all| live(all) && twice(&requests(&bed.server))).await;
    let listed = before.iter().filter(|(lists, _, _)| *lists == 1).count();
    assert_eq!(listed, STORES, "one list of each store before the change: {before:?}");
}

/// A new cluster at the same address can pass the checks of each store after a lost
/// connection: its versions can be past the cursors, and its first objects can have no
/// namesake in the stores. Its Namespace kube-system has an other UID, and each store lists
/// again; a lost connection to the same cluster costs no list.
#[tokio::test]
async fn an_other_uid_of_kube_system_after_a_lost_connection_gives_each_store_a_new_list() {
    let bed = ProxyBed::start("new-cluster", NetSettings::default()).await;
    bed.server.set_body(KUBE_SYSTEM, &body("first"));
    let stores = Stores::start(&bed.tasks, &bed.link, StoreSettings::default());
    stores.until_live(LIMIT).await;
    let server = &bed.server;
    in_time("the read at the connect", reads(server, 1)).await;
    let lists =
        || -> Vec<usize> { Kind::ALL.iter().map(|kind| lists_and_watches(server, &kind.info().path()).0).collect() };
    let first = lists();
    assert_eq!(first.iter().filter(|count| **count == 1).count(), STORES, "one list for each store: {first:?}");

    let lost = SystemTime::now();
    in_time("a reset", bed.proxy.reset_all()).await;
    in_time("the read after the lost connection", reads(server, 2)).await;
    live_again_after(&stores, lost, RECOVERY).await;
    assert_eq!(lists(), first, "the same cluster: no list");

    bed.server.set_body(KUBE_SYSTEM, &body("second"));
    let lost = SystemTime::now();
    in_time("a reset", bed.proxy.reset_all()).await;
    let again = |all: &[StoreStatus]| all.iter().all(|store| store.state.is_live() && store.since >= lost);
    stores
        .until("each store is live after a new list", RECOVERY, |all| {
            again(all) && lists().iter().zip(&first).all(|(now, before)| *now == before * 2)
        })
        .await;
}
