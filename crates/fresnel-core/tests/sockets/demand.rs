//! The stores on demand on the fake API server (plan section 9.5): the start with the
//! first holder, the stop after the time of the settings, and the states of a kind that the
//! server refuses or does not serve.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use fresnel_core::kinds::Kind;
use fresnel_core::net::NetSettings;
use fresnel_core::published::Reader;
use fresnel_core::store::{EventsMode, ObjectKey, Snapshot, StoreSettings, StoreState, StoreStatus};
use fresnel_testkit::Fault;
use http::header::ACCEPT;
use k8s_openapi::api::coordination::v1::Lease;
use k8s_openapi::api::core::v1::{ConfigMap, Event, Pod, Secret};

use crate::bed::Bed;
use crate::common::{LIMIT, in_time};
use crate::store_bed::Stores;
use crate::stores::{PODS, STORES, of_path, state_of};

/// A short time, so that a test waits for the end of a store in real time.
const LINGER: Duration = Duration::from_millis(400);

fn lists(bed: &Bed, path: &str) -> usize {
    of_path(&bed.server.requests(), path).iter().filter(|seen| !seen.is_watch()).count()
}

/// Waits until the store shows a state that meets the condition.
async fn until<T: Send + Sync + 'static>(
    reader: &Reader<Snapshot<T>>,
    what: &str,
    met: impl Fn(&StoreState) -> bool,
) -> StoreState {
    let mut reader = reader.clone();
    in_time(what, async {
        loop {
            let state = reader.read().state.clone();
            if met(&state) {
                return state;
            }
            reader.changed().await;
        }
    })
    .await
}

#[tokio::test]
async fn a_store_on_demand_starts_with_its_first_holder_and_stops_its_time_after_the_last_one() {
    let bed = Bed::start(NetSettings::default()).await;
    let leases = Kind::Lease.info().path();
    let lease = r#"{"metadata":{"name":"k3s","namespace":"kube-system","resourceVersion":"5"},"spec":{"holderIdentity":"k3s-1"}}"#;
    bed.server.set_body(&leases, &format!(r#"{{"metadata":{{"resourceVersion":"7"}},"items":[{lease}]}}"#));
    let settings = StoreSettings { linger: LINGER, ..StoreSettings::default() };
    let stores = Stores::start(&bed.tasks, &bed.link, settings);
    stores.until_live(LIMIT).await;
    assert_eq!(lists(&bed, &leases), 0, "a tier C kind has no store that is always on");

    let first = stores.stores.demand::<Lease>().expect("a store of the Leases");
    let reader = first.reader().clone();
    until(&reader, "the store on demand is live", StoreState::is_live).await;
    let snapshot = first.read();
    let stored = snapshot.items.get(&ObjectKey::new("kube-system", "k3s")).expect("the Lease");
    let holder = stored.spec.as_ref().and_then(|spec| spec.holder_identity.as_deref());
    assert_eq!(holder, Some("k3s-1"), "a store on demand has the full objects");
    let requests = bed.server.requests();
    assert!(of_path(&requests, &leases).iter().all(|seen| !seen.headers.contains_key(ACCEPT)), "no metadata form");
    drop(snapshot);

    // A second holder keeps the store after the first one went.
    let second = first.clone();
    drop(first);
    tokio::time::sleep(LINGER * 3).await;
    assert!(reader.read().state.is_live(), "a holder lives");

    // A view that opens again in the time takes the same store, with no new list.
    drop(second);
    let third = stores.stores.demand::<Lease>().expect("the same store");
    tokio::time::sleep(LINGER * 3).await;
    assert!(reader.read().state.is_live());
    assert_eq!(lists(&bed, &leases), 1);

    let went = Instant::now();
    drop(third);
    until(&reader, "the store stops", |state| *state == StoreState::Stopped).await;
    let after = went.elapsed();
    assert!(
        after >= LINGER && after < LINGER + Duration::from_secs(5),
        "the store stopped {after:?} after the last holder"
    );
    assert_eq!(reader.read().items.len(), 1, "a stopped store keeps its objects");
    let stored_kinds = u64::try_from(STORES).expect("a small count");
    in_time("the watch of the Leases closes", bed.server.wait_for(|counters| counters.open_watches == stored_kinds))
        .await;

    // The next holder starts a new store with a new list.
    let again = stores.stores.demand::<Lease>().expect("a new store");
    until(again.reader(), "the new store is live", StoreState::is_live).await;
    assert_eq!(lists(&bed, &leases), 2);

    // The stop of the stores of the cluster stops a store on demand that has a holder.
    drop(stores);
    assert_eq!(again.read().state, StoreState::Stopped);
    in_time("each watch closes", bed.server.wait_for(|counters| counters.open_watches == 0)).await;
}

/// Plan section 9.4: no store holds the value of a Secret, also no store on demand.
#[tokio::test]
async fn a_store_on_demand_of_secrets_keeps_their_keys_and_no_value_and_a_tier_a_kind_gives_its_own_store() {
    let bed = Bed::start(NetSettings::default()).await;
    let secret = r#"{"metadata":{"name":"db","namespace":"shop","resourceVersion":"3"},"type":"Opaque","data":{"password":"bm90LWEtc2VjcmV0"}}"#;
    bed.server.set_body("/api/v1/secrets", &format!(r#"{{"metadata":{{"resourceVersion":"7"}},"items":[{secret}]}}"#));
    let settings = StoreSettings { events: EventsMode::OnDemand, ..StoreSettings::default() };
    let stores = Stores::start(&bed.tasks, &bed.link, settings);
    stores.until_live(LIMIT).await;

    let secrets = stores.stores.demand::<Secret>().expect("a store of the Secrets");
    until(secrets.reader(), "the store of the Secrets is live", StoreState::is_live).await;
    let snapshot = secrets.read();
    let stored = snapshot.items.get(&ObjectKey::new("shop", "db")).expect("the Secret");
    let data = stored.data.as_ref().expect("the keys");
    assert_eq!(data.keys().collect::<Vec<_>>(), ["password"]);
    assert!(data.values().all(|value| value.0.is_empty()), "no value");
    assert_eq!(stored.type_.as_deref(), Some("Opaque"));
    assert!(stores.stores.metadata(Kind::Secret).is_some(), "the store of the metadata stays");

    let pods_before = lists(&bed, PODS);
    let pods = stores.stores.demand::<Pod>().expect("the store of the Pods");
    assert!(pods.read().state.is_live() && lists(&bed, PODS) == pods_before, "the store that is always on");
    assert!(
        stores.stores.full::<ConfigMap>().is_none(),
        "a tier B kind has no store of full objects that is always on"
    );

    // With the setting "on demand", the Events have a store while a view needs them.
    assert!(stores.stores.full::<Event>().is_none());
    let events = stores.stores.demand::<Event>().expect("a store of the Events");
    until(events.reader(), "the store of the Events is live", StoreState::is_live).await;
    assert_eq!(events.read().items.len(), 0);
    drop(snapshot);
}

/// Plan section 9.7: a kind that the server does not serve shows "not supported", HTTP 403
/// for a kind shows "forbidden", and each other store stays live. A store on demand has
/// the same states.
#[tokio::test]
async fn a_kind_that_the_server_does_not_serve_or_refuses_shows_its_state_and_the_other_kinds_stay_live() {
    let bed = Bed::start(NetSettings::default()).await;
    let (classes, leases) = (Kind::IngressClass.info().path(), Kind::Lease.info().path());
    bed.server.add_fault(Fault::not_found().on_path(&classes));
    bed.server.add_fault(Fault::not_found().on_path(&leases));
    bed.server.add_fault(Fault::forbidden().on_path("/api/v1/secrets"));
    let stores = Stores::start(&bed.tasks, &bed.link, StoreSettings::default());
    let refused = |state: &StoreState| matches!(state, StoreState::NotSupported { .. } | StoreState::Forbidden { .. });
    let settled = |all: &[StoreStatus]| {
        all.iter().all(|status| status.state.is_live() || refused(&status.state))
            && all.iter().filter(|status| status.state.is_live()).count() == STORES - 2
    };
    let statuses = stores.until("each other store is live", LIMIT, settled).await;
    assert_eq!(
        state_of(&statuses, Kind::IngressClass).to_string(),
        "not supported: the server could not find the requested resource (HTTP 404)"
    );
    assert_eq!(
        state_of(&statuses, Kind::Secret).to_string(),
        "forbidden: forbidden: the user of the test cannot read this path (HTTP 403)"
    );

    let leases_store = stores.stores.demand::<Lease>().expect("a store of the Leases");
    let state =
        until(leases_store.reader(), "the refusal", |state| matches!(state, StoreState::NotSupported { .. })).await;
    assert_eq!(state.to_string(), "not supported: the server could not find the requested resource (HTTP 404)");
    let secrets = stores.stores.demand::<Secret>().expect("a store of the Secrets");
    until(secrets.reader(), "the refusal", |state| matches!(state, StoreState::Forbidden { .. })).await;
    let config_maps = stores.stores.demand::<ConfigMap>().expect("a store of the ConfigMaps");
    until(config_maps.reader(), "the store of the ConfigMaps is live", StoreState::is_live).await;
    let live = stores.stores.statuses().iter().filter(|status| status.state.is_live()).count();
    assert_eq!(live, STORES - 2, "a refused store on demand changes no other store");
}

/// High load: the holders of one kind come and go on many threads, and the time of the
/// settings is zero, so the last holder that goes stops the store and the next one starts
/// a new store. A store never stops while a holder lives, and the stores that stopped
/// leave no task, no list slot and no watch.
#[tokio::test(flavor = "multi_thread")]
async fn holders_that_come_and_go_on_many_threads_never_stop_a_held_store_and_leave_nothing() {
    const THREADS: usize = 8;
    const ROUNDS: usize = 200;
    let bed = Bed::start(NetSettings::default()).await;
    let settings = StoreSettings { linger: Duration::ZERO, ..StoreSettings::default() };
    let stores = Stores::start(&bed.tasks, &bed.link, settings);
    stores.until_live(LIMIT).await;
    let tasks = bed.tasks.tasks.snapshot().len();

    let stopped_while_held = AtomicUsize::new(0);
    tokio::task::block_in_place(|| {
        std::thread::scope(|scope| {
            for _ in 0..THREADS {
                scope.spawn(|| {
                    for _ in 0..ROUNDS {
                        let held = stores.stores.demand::<Lease>().expect("a store of the Leases");
                        let second = held.clone();
                        drop(held);
                        if second.read().state == StoreState::Stopped {
                            stopped_while_held.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                });
            }
        });
    });
    assert_eq!(stopped_while_held.into_inner(), 0, "a store that a holder has never stops");

    let last = stores.stores.demand::<Lease>().expect("a store of the Leases");
    until(last.reader(), "the next store is live: no list slot stays taken", StoreState::is_live).await;
    drop(last);
    let stored_kinds = u64::try_from(STORES).expect("a small count");
    in_time(
        "each watch of a store on demand closes",
        bed.server.wait_for(|counters| counters.open_watches == stored_kinds),
    )
    .await;
    // A store leaves the list of the tasks before its task stops, and so before its watch.
    assert_eq!(bed.tasks.tasks.snapshot().len(), tasks, "each task of a store on demand ended");
    assert!(stores.stores.statuses().iter().all(|status| status.state.is_live()), "{:#?}", stores.states());
}
