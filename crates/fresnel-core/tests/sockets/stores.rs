//! The stores of one cluster on the fake API server: the requests of the watch loops, the
//! way of an event to a reader, and the states for a server that refuses.

use std::time::{Duration, Instant};

use fresnel_core::kinds::{Kind, Tier};
use fresnel_core::net::NetSettings;
use fresnel_core::store::{EventsMode, ObjectKey, StoreSettings, StoreState, StoreStatus};
use fresnel_testkit::{Fault, SeenRequest};
use http::header::{ACCEPT, ACCEPT_ENCODING};
use k8s_openapi::api::core::v1::{Event, Pod};

use crate::bed::{Bed, ProxyBed};
use crate::common::{LIMIT, in_time};
use crate::store_bed::Stores;

pub(crate) const PODS: &str = "/api/v1/pods";
const SECRETS: &str = "/api/v1/secrets";
/// The kinds of tier A and tier B.
pub(crate) const STORES: usize = 39;

pub(crate) fn of_path<'a>(requests: &'a [SeenRequest], path: &str) -> Vec<&'a SeenRequest> {
    requests.iter().filter(|seen| seen.uri.path() == path).collect()
}

pub(crate) fn state_of(statuses: &[StoreStatus], kind: Kind) -> &StoreState {
    &statuses.iter().find(|status| status.kind == kind).expect("the store of the kind").state
}

/// Plan section 15: 30 watches use 3 connections or fewer. The standard client opened 31.
#[tokio::test]
async fn the_stores_of_each_kind_are_live_on_three_connections() {
    let bed = Bed::start(NetSettings::default()).await;
    let stores = Stores::start(&bed.tasks, &bed.link, StoreSettings::default());
    let statuses = stores.until_live(LIMIT).await;
    let kinds: Vec<Kind> = statuses.iter().map(|status| status.kind).collect();
    let expected: Vec<Kind> = Kind::ALL.iter().copied().filter(|kind| kind.info().tier != Tier::C).collect();
    assert_eq!((kinds.len(), &kinds), (STORES, &expected), "one store for each kind of tier A and tier B");

    let counters = in_time("each watch is open", bed.server.wait_for(|counters| counters.open_watches == 39)).await;
    assert_eq!(counters.connections, 3, "39 lists and 39 watches used the three channels of the cluster");
    let status = bed.link.net.status();
    assert_eq!((status.watches.channels.len(), status.watches.channels[0].streams_open), (1, STORES));

    // A tier A kind asks for full objects, a tier B kind for the metadata form. A list
    // asks for gzip, and a watch does not: its lines must arrive one by one.
    let requests = bed.server.requests();
    let (pods, secrets) = (of_path(&requests, PODS), of_path(&requests, SECRETS));
    assert_eq!((pods.len(), secrets.len()), (2, 2), "one list and one watch for each kind");
    assert_eq!(pods[0].uri.query(), Some("&limit=500"));
    assert_eq!(
        (pods[0].headers.get(ACCEPT), &pods[0].headers[ACCEPT_ENCODING]),
        (None, &"gzip".parse().expect("a value"))
    );
    assert_eq!(secrets[0].headers[ACCEPT], "application/json;as=PartialObjectMetadataList;g=meta.k8s.io;v=v1");
    assert_eq!(secrets[1].headers[ACCEPT], "application/json;as=PartialObjectMetadata;g=meta.k8s.io;v=v1");
    for watch in [pods[1], secrets[1]] {
        let query: Vec<&str> = watch.uri.query().expect("a query").split('&').collect();
        assert!(watch.is_watch() && !watch.headers.contains_key(ACCEPT_ENCODING), "{:?}", watch.uri);
        // The watch starts after the version of the list, and the server ends it in 60 to 90 s.
        assert!(query.contains(&"resourceVersion=1") && query.contains(&"allowWatchBookmarks=true"), "{query:?}");
        let timeout = query.iter().find_map(|pair| pair.strip_prefix("timeoutSeconds=")).expect("a timeout");
        assert!((60..=90).contains(&timeout.parse::<u32>().expect("a number")), "{timeout}");
    }
    for off in [Kind::Lease, Kind::Endpoints] {
        assert_eq!(of_path(&requests, &off.info().path()).len(), 0, "{off:?} changes all the time and has no store");
    }
}

#[tokio::test]
async fn an_event_of_the_server_reaches_each_reader_and_a_stopped_store_keeps_its_objects() {
    let bed = Bed::start(NetSettings::default()).await;
    let pod = |name: &str, version: &str| {
        format!(
            r#"{{"metadata":{{"name":"{name}","namespace":"shop","uid":"u-{name}","resourceVersion":"{version}"}}}}"#
        )
    };
    let list = |items: &str| format!(r#"{{"metadata":{{"resourceVersion":"7"}},"items":[{items}]}}"#);
    bed.server.set_body(PODS, &list(&format!("{},{}", pod("api", "5"), pod("web", "6"))));
    // The metadata form of a Secret that `kubectl apply` made: the annotation holds its values.
    let applied = r#"{"metadata":{"name":"db","namespace":"shop","resourceVersion":"3","managedFields":[{"manager":"kubectl"}],
        "annotations":{"kubectl.kubernetes.io/last-applied-configuration":"{\"data\":{\"password\":\"c2VjcmV0\"}}"}}}"#;
    bed.server.set_body(SECRETS, &list(applied));

    let stores = Stores::start(&bed.tasks, &bed.link, StoreSettings::default());
    stores.until_live(LIMIT).await;
    let mut pods = stores.stores.full::<Pod>().expect("the store of the pods");
    let names = |snapshot: &fresnel_core::store::Snapshot<Pod>| {
        snapshot.items.iter().map(|(key, _)| key.name.to_string()).collect::<Vec<_>>()
    };
    let first = pods.read();
    assert_eq!((names(&first), first.generation), (vec!["api".to_owned(), "web".to_owned()], 1));

    let event = |kind: &str, object: &str| format!(r#"{{"type":"{kind}","object":{object}}}"#);
    assert_eq!(bed.server.send_event(PODS, &event("ADDED", &pod("job", "8"))), STORES);
    bed.server.send_event(PODS, &event("DELETED", &pod("api", "9")));
    let changed = in_time("the two events", async {
        loop {
            let snapshot = pods.read();
            if names(&snapshot) == ["job", "web"] {
                return snapshot;
            }
            pods.changed().await;
        }
    });
    let changed = changed.await;
    assert!(changed.last_contact > first.last_contact && changed.state.is_live());
    assert_eq!(names(&first), ["api", "web"], "a snapshot that a reader holds does not change");

    let secrets = stores.stores.metadata(Kind::Secret).expect("the store of the secrets");
    let secret = secrets.read();
    let stored = secret.items.get(&ObjectKey::new("shop", "db")).expect("the secret");
    assert_eq!((&stored.metadata.annotations, &stored.metadata.managed_fields), (&None, &None));
    assert!(stores.stores.metadata(Kind::Pod).is_none(), "a tier A kind has no metadata store");
    assert_eq!(stores.stores.cluster().as_str(), "bed");

    // The stop of the stores closes each watch. A reader keeps the objects, with the state.
    drop(stores);
    let stopped = pods.read();
    assert_eq!((names(&stopped), &stopped.state), (vec!["job".to_owned(), "web".to_owned()], &StoreState::Stopped));
    in_time("each watch closes", bed.server.wait_for(|counters| counters.open_watches == 0)).await;
    let tasks: Vec<String> = bed.tasks.tasks.snapshot().iter().map(|task| task.name.to_string()).collect();
    assert_eq!(tasks.len(), 4, "the tasks of the connections stay: {tasks:?}");
    assert!(tasks.iter().all(|task| !task.starts_with("store") && task != "old snapshots"), "{tasks:?}");
}

/// Plan section 9.6: Events can be most of the objects of a cluster.
#[tokio::test]
async fn the_settings_for_events_select_warnings_only_or_no_store_and_limit_their_count() {
    let bed = Bed::start(NetSettings::default()).await;
    let events = Kind::Event.info().path();
    let event = |name: &str, second: u8| {
        let time = format!("2026-01-01T00:00:{second:02}Z");
        format!(
            r#"{{"metadata":{{"name":"{name}","namespace":"shop","resourceVersion":"3","creationTimestamp":"{time}"}},
            "involvedObject":{{}},"type":"Warning"}}"#
        )
    };
    let list = format!(r#"{{"metadata":{{"resourceVersion":"7"}},"items":[{},{}]}}"#, event("new", 9), event("old", 1));
    bed.server.set_body(&events, &list);
    let warnings = StoreSettings { events: EventsMode::Warnings, max_events: 1, ..StoreSettings::default() };
    let stores = Stores::start(&bed.tasks, &bed.link, warnings);
    let statuses = stores.until_live(LIMIT).await;
    assert_eq!(statuses.len(), STORES);
    let kept = stores.stores.full::<Event>().expect("the store of the events").read();
    let names: Vec<&str> = kept.items.iter().map(|(key, _)| &*key.name).collect();
    assert_eq!((names, kept.dropped), (vec!["new"], 1), "the newest Event stays, and the count tells the other one");
    assert_eq!(statuses.iter().map(|status| status.dropped).sum::<u64>(), 1, "no other store has a limit");
    let requests = bed.server.requests();
    for request in of_path(&requests, &events) {
        let query = request.uri.query().expect("a query");
        assert!(query.split('&').any(|pair| pair == "fieldSelector=type%3DWarning"), "the server selects: {query}");
    }
    assert!(of_path(&requests, PODS).iter().all(|seen| !seen.uri.query().unwrap_or_default().contains("Selector")));
    drop((kept, stores));

    let on_demand = StoreSettings { events: EventsMode::OnDemand, ..StoreSettings::default() };
    let stores = Stores::start(&bed.tasks, &bed.link, on_demand);
    assert_eq!(stores.until_live(LIMIT).await.len(), STORES - 1);
    assert!(stores.stores.full::<Event>().is_none());
    assert_eq!(of_path(&bed.server.requests(), &events).len(), 2, "no request for Events after the first stores");
}

#[tokio::test]
async fn a_server_that_refuses_or_throttles_shows_as_the_state_of_the_store() {
    let bed = Bed::start(NetSettings::default()).await;
    bed.server.add_fault(Fault::forbidden().on_path(SECRETS));
    let config_maps = Kind::ConfigMap.info().path();
    let asked = Duration::from_secs(2);
    bed.server.add_fault(Fault::too_many_requests(asked).on_path(&config_maps).for_time(asked / 2));

    let started = (Instant::now(), std::time::SystemTime::now());
    let stores = Stores::start(&bed.tasks, &bed.link, StoreSettings::default());
    let is_throttled = |state: &StoreState| matches!(state, StoreState::Throttled { .. });
    let throttled = stores.until("the throttle", LIMIT, |all| is_throttled(state_of(all, Kind::ConfigMap))).await;
    let StoreState::Throttled { until } = state_of(&throttled, Kind::ConfigMap) else { panic!("throttled") };
    let wait = until.duration_since(started.1).expect("a time after the start");
    assert!(asked <= wait + Duration::from_millis(50) && wait < asked * 2, "the time of `Retry-After`: {wait:?}");

    // The connection layer sends the list again after that time. Then each store is live
    // but the one that the server refuses.
    let settled = |all: &[StoreStatus]| all.iter().filter(|status| status.state.is_live()).count() == STORES - 1;
    let statuses = stores.until("each other store is live", LIMIT, settled).await;
    assert!(started.0.elapsed() >= asked, "no request went out before the time of the server");
    assert_eq!(
        state_of(&statuses, Kind::Secret).to_string(),
        "forbidden: forbidden: the user of the test cannot read this path (HTTP 403)"
    );
    let requests = bed.server.requests();
    let lists = |path: &str| of_path(&requests, path).iter().filter(|seen| !seen.is_watch()).count();
    assert_eq!((lists(&config_maps), lists(SECRETS)), (2, 1), "one repeat after the throttle, none after the refusal");
}

/// Plan section 6.8: old data must not show as current. A connection that carries no data
/// and gives no error shows in each store in 20 seconds or less, and the stores continue
/// on a new connection with no new list.
#[tokio::test]
async fn a_silent_connection_shows_in_each_store_in_20_seconds_or_less() {
    let bed = ProxyBed::start("silent-stores", NetSettings::default()).await;
    let stores = Stores::start(&bed.tasks, &bed.link, StoreSettings::default());
    stores.until_live(LIMIT).await;

    bed.proxy.silence_open();
    let silent = Instant::now();
    let none_live = |all: &[StoreStatus]| all.iter().all(|status| !status.state.is_live());
    let statuses = stores.until("no store is live", Duration::from_secs(20), none_live).await;
    println!("each store showed the silent connection after {:?}", silent.elapsed());
    // The channel counts the loss. A store counts no failure of its own and takes no pause.
    let paused = statuses.iter().filter(|status| matches!(status.state, StoreState::Backoff { .. })).count();
    assert_eq!(paused, 0, "{:#?}", stores.states());
    assert!(statuses.iter().all(|status| status.since.elapsed().is_ok_and(|age| age < Duration::from_secs(1))));

    let found = Instant::now();
    stores.until_live(Duration::from_secs(10)).await;
    println!("each store was live again {:?} after that", found.elapsed());
    let lists = bed.server.requests().iter().filter(|seen| seen.uri.path() != "/version" && !seen.is_watch()).count();
    assert_eq!(lists, STORES, "each watch continued from its last version: no new list");
    assert_eq!(bed.proxy.task_failures(), Vec::<String>::new());
}
