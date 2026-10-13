//! The stores of one cluster with faults that the acceptance of milestone M1 does not name:
//! a second fault during a recovery, a fault at the worst moment, a server that does not
//! answer, and bad data of the server.

use std::time::{Duration, Instant, SystemTime};

use fresnel_core::kinds::Kind;
use fresnel_core::net::{Limits, NetSettings};
use fresnel_core::store::{Snapshot, StoreSettings, StoreState, StoreStatus};
use fresnel_testkit::{FakeApiServer, Fault, ProxyMode};
use k8s_openapi::api::core::v1::Pod;

use crate::bed::{Bed, ProxyBed};
use crate::common::{LIMIT, in_time};
use crate::store_bed::Stores;
use crate::stores::{PODS, STORES, of_path, state_of};

/// Plan section 15: each store is live 10 seconds or less after the network returns.
const RECOVERY: Duration = Duration::from_secs(10);
/// Plan section 15: rows that are not live get their mark in 20 seconds or less.
const SHOWS_IN: Duration = Duration::from_secs(20);

/// A pod whose list of containers is a text, so it does not fit the type of a pod. The
/// text stands for a value of an object: no error text must show it.
const BAD_POD: &str = r#"{"metadata":{"name":"bad","resourceVersion":"9"},"spec":{"containers":"c2VjcmV0"}}"#;
/// The `ERROR` event of a server that no longer has the version of a watch.
const EXPIRED: &str = concat!(
    r#"{"type":"ERROR","object":{"kind":"Status","apiVersion":"v1","status":"Failure","#,
    r#""message":"too old resource version: 7 (31)","reason":"Expired","code":410}}"#
);

/// One pod of the namespace `shop`, as the server sends it.
fn pod(name: &str, version: &str) -> String {
    let metadata = format!(r#""name":"{name}","namespace":"shop","uid":"u-{name}","resourceVersion":"{version}""#);
    format!(r#"{{"metadata":{{{metadata}}}}}"#)
}

/// The body of a list with the version `version`.
fn list(items: &[String], version: &str) -> String {
    format!(r#"{{"metadata":{{"resourceVersion":"{version}"}},"items":[{}]}}"#, items.join(","))
}

/// The body of a list of pods, each one as name and resource version.
fn pod_list(pods: &[(&str, &str)], version: &str) -> String {
    let items: Vec<String> = pods.iter().map(|(name, version)| pod(name, version)).collect();
    list(&items, version)
}

fn pod_names(snapshot: &Snapshot<Pod>) -> Vec<String> {
    snapshot.items.iter().map(|(key, _)| key.name.to_string()).collect()
}

/// The requests of one path that the server saw, lists and watches apart.
fn lists_and_watches(server: &FakeApiServer, path: &str) -> (usize, usize) {
    let requests = server.requests();
    let of_path = of_path(&requests, path);
    let watches = of_path.iter().filter(|seen| seen.is_watch()).count();
    (of_path.len() - watches, watches)
}

/// Waits until each store is live with a state from after `fault`. Gives the time from
/// `fault` to the last of these states.
async fn live_again_after(stores: &Stores, fault: SystemTime, limit: Duration) -> Duration {
    let again = |all: &[StoreStatus]| all.iter().all(|store| store.state.is_live() && store.since >= fault);
    let all = stores.until("each store is live again", limit, again).await;
    let last = all.iter().map(|store| store.since).max().unwrap_or(fault);
    last.duration_since(fault).unwrap_or_default()
}

/// A second fault during a recovery. Each connection gets a reset, and a second reset
/// comes while the app connects again. Then the version of the watch of the pods is gone
/// two times: the second HTTP 410 comes for the watch after the new list.
#[tokio::test]
async fn a_reset_during_a_recovery_costs_no_list_and_each_http_410_costs_one() {
    let bed = ProxyBed::start("second-fault", NetSettings::default()).await;
    bed.server.set_body(PODS, &pod_list(&[("api", "5"), ("web", "6")], "7"));
    bed.proxy.set(ProxyMode::Latency(Duration::from_millis(20)));
    let stores = Stores::start(&bed.tasks, &bed.link, StoreSettings::default());
    stores.until_live(LIMIT).await;

    let connections = bed.server.counters().connections;
    in_time("the first reset", bed.proxy.reset_all()).await;
    in_time("a new connection", bed.server.wait_for(|counters| counters.connections > connections)).await;
    in_time("the second reset", bed.proxy.reset_all()).await;
    let recovery = live_again_after(&stores, SystemTime::now(), RECOVERY).await;
    println!("each store was live {recovery:?} after the second of two resets");

    bed.server.set_body(PODS, &pod_list(&[("api", "5"), ("job", "30"), ("web", "6")], "31"));
    for round in 1..=2 {
        let (lists, started) = (lists_and_watches(&bed.server, PODS).0, Instant::now());
        bed.server.send_event(PODS, EXPIRED);
        in_time("a new list of the pods", bed.server.wait_for(|_| lists_and_watches(&bed.server, PODS).0 > lists))
            .await;
        stores.until_live(RECOVERY).await;
        println!("HTTP 410 number {round}: the store of the pods was live again after {:?}", started.elapsed());
    }

    let pods = stores.stores.full::<Pod>().expect("the store of the pods").read();
    assert_eq!(pod_names(&pods), ["api", "job", "web"], "the list after HTTP 410 replaced the objects");
    assert_eq!(lists_and_watches(&bed.server, PODS).0, 3, "one list at the start and one for each HTTP 410");
    let requests = bed.server.requests();
    let last_watch = of_path(&requests, PODS).into_iter().rfind(|seen| seen.is_watch()).expect("a watch");
    let query = last_watch.uri.query().expect("a query");
    assert!(query.split('&').any(|pair| pair == "resourceVersion=31"), "the watch follows the new list: {query}");
    for kind in Kind::ALL.iter().filter(|kind| **kind != Kind::Pod) {
        let lists = lists_and_watches(&bed.server, &kind.info().path()).0;
        assert!(lists <= 1, "a reset costs no new list, and {kind:?} has {lists} lists");
    }
    assert_eq!(bed.proxy.task_failures(), Vec::<String>::new());
}

/// A fault at the worst moment. The server ends each watch at its timeout, and the store
/// stays live while the next one starts. In that moment each open connection goes silent:
/// the request of the new watch gets no answer and no error.
#[tokio::test]
async fn a_connection_that_goes_silent_at_the_renewal_of_each_watch_shows_in_20_seconds_or_less() {
    let bed = ProxyBed::start("silent-renewal", NetSettings::default()).await;
    let stores = Stores::start(&bed.tasks, &bed.link, StoreSettings::default());
    let started = stores.until_live(LIMIT).await;
    // A bookmark shows each store that its watch works. Only the end of such a watch is
    // the usual end at the timeout, and not a failure.
    let bookmark =
        r#"{"type":"BOOKMARK","object":{"kind":"Pod","apiVersion":"v1","metadata":{"resourceVersion":"2"}}}"#;
    for kind in started.iter().map(|store| store.kind) {
        bed.server.send_event(&kind.info().path(), bookmark);
    }
    let marked = |all: &[StoreStatus]| all.iter().zip(&started).all(|(now, then)| now.last_contact > then.last_contact);
    let before = stores.until("the bookmark of each watch", LIMIT, marked).await;
    // With this latency, the answer to a new watch is on its way for 100 ms.
    bed.proxy.set(ProxyMode::Latency(Duration::from_millis(100)));

    let watches = |server: &FakeApiServer| server.requests().iter().filter(|seen| seen.is_watch()).count();
    let open = watches(&bed.server);
    assert_eq!(bed.server.end_watches(), STORES);
    in_time("the request of a new watch", bed.server.wait_for(|_| watches(&bed.server) > open)).await;
    bed.proxy.silence_open();
    let silent = Instant::now();
    let renewing = stores.stores.statuses();
    assert_eq!(renewing, before, "each store is live with no open watch: the worst moment for a silent connection");

    let none_live = |all: &[StoreStatus]| all.iter().all(|store| !store.state.is_live());
    let shown = stores.until("no store is live", SHOWS_IN, none_live).await;
    println!("each store showed the silent connection after {:?}: {}", silent.elapsed(), state_of(&shown, Kind::Pod));
    let found = SystemTime::now();
    let recovery = live_again_after(&stores, found, RECOVERY).await;
    println!("each store was live again {recovery:?} after that");
    let lists = bed.server.requests().iter().filter(|seen| seen.uri.path() != "/version" && !seen.is_watch()).count();
    assert_eq!(lists, STORES, "each watch continued from its last version: no new list");
    assert_eq!(bed.proxy.task_failures(), Vec::<String>::new());
}

/// A slow peer. The server gives no answer to the lists of eight kinds, and these are
/// kinds with a high priority. Each such list holds one of the four list slots of the
/// cluster to its time limit, and then it asks again.
#[tokio::test]
async fn kinds_whose_lists_get_no_answer_do_not_keep_the_list_slots_from_the_other_kinds() {
    let limits = Limits { list_idle: Duration::from_secs(1), ..Limits::default() };
    let bed = Bed::start(NetSettings { limits, ..NetSettings::default() }).await;
    let slow = [
        Kind::Namespace,
        Kind::Node,
        Kind::Deployment,
        Kind::StatefulSet,
        Kind::DaemonSet,
        Kind::ReplicaSet,
        Kind::Job,
        Kind::CronJob,
    ];
    for kind in slow {
        bed.server.add_fault(Fault::no_answer().on_path(kind.info().path()));
    }
    let started = Instant::now();
    let stores = Stores::start(&bed.tasks, &bed.link, StoreSettings::default());
    // Two rounds of four lists with no answer come first. Then each other kind has its turn.
    let others_live = |all: &[StoreStatus]| all.iter().all(|store| slow.contains(&store.kind) != store.state.is_live());
    let statuses = stores.until("each other store is live", Duration::from_secs(6), others_live).await;
    println!("the {} stores with an answer were live after {:?}", STORES - slow.len(), started.elapsed());
    for kind in slow {
        println!("  {}: {}", kind.info().name, state_of(&statuses, kind));
    }

    bed.server.clear_faults();
    let healed = Instant::now();
    stores.until_live(RECOVERY).await;
    println!("each store was live {:?} after the server answered again", healed.elapsed());
}

/// Bad data from the cluster in a list: an object that does not fit its type, and a list
/// with no end. The store of that kind shows the failure and asks again with a pause.
/// Each other store is live.
#[tokio::test]
async fn bad_data_in_a_list_stops_the_store_of_its_kind_only_and_does_not_load_the_server() {
    let bed = Bed::start(NetSettings::default()).await;
    let config_maps = Kind::ConfigMap.info().path();
    bed.server.set_body(PODS, &list(&[BAD_POD.to_owned()], "7"));
    // The fake server gives this page for each token: a list that never ends.
    bed.server.set_body(&config_maps, r#"{"metadata":{"resourceVersion":"7","continue":"again"},"items":[]}"#);

    let stores = Stores::start(&bed.tasks, &bed.link, StoreSettings::default());
    let bad = [Kind::Pod, Kind::ConfigMap];
    let failed = |state: &StoreState| matches!(state, StoreState::Backoff { failures, .. } if *failures >= 3);
    let settled = |all: &[StoreStatus]| {
        all.iter().all(|store| if bad.contains(&store.kind) { failed(&store.state) } else { store.state.is_live() })
    };
    let statuses = stores.until("the two stores with bad data failed three times", LIMIT, settled).await;
    let (pods, maps) = (state_of(&statuses, Kind::Pod).to_string(), state_of(&statuses, Kind::ConfigMap).to_string());
    println!("{pods}\n{maps}");
    assert!(pods.contains("JSON error at line 1, column") && !pods.contains("c2VjcmV0"), "{pods}");
    assert!(maps.ends_with("the list of the server gave the token of a page two times"), "{maps}");
    // Three failures are three lists of the pods, and two requests for each list with no end.
    let requests = (lists_and_watches(&bed.server, PODS).0, lists_and_watches(&bed.server, &config_maps).0);
    assert!(requests.0 <= 4 && requests.1 <= 8, "the pause after each failure limits the requests: {requests:?}");

    // The data is good again. The store is live with no restart of the app.
    bed.server.set_body(PODS, &pod_list(&[("api", "5")], "7"));
    bed.server.set_body(&config_maps, r#"{"metadata":{"resourceVersion":"7"},"items":[]}"#);
    stores.until_live(RECOVERY).await;
    assert_eq!(pod_names(&stores.stores.full::<Pod>().expect("the store of the pods").read()), ["api"]);
}

/// Bad data from the cluster in a watch: a line that is not JSON, an object that does not
/// fit its type, and an object with no name. The watch starts again after the last version
/// that the app could read, with no new list, and the next good event arrives.
#[tokio::test]
async fn a_bad_line_of_a_watch_starts_the_watch_again_after_the_last_good_version() {
    let bed = Bed::start(NetSettings::default()).await;
    bed.server.set_body(PODS, &pod_list(&[("api", "5")], "7"));
    let stores = Stores::start(&bed.tasks, &bed.link, StoreSettings::default());
    let before = stores.until_live(LIMIT).await;
    let mut pods = stores.stores.full::<Pod>().expect("the store of the pods");
    let added = |object: &str| format!(r#"{{"type":"ADDED","object":{object}}}"#);
    let bad_lines = [
        r#"{"type":"ADDED","object":"#.to_owned(),
        added(BAD_POD),
        added(r#"{"metadata":{"namespace":"shop","resourceVersion":"10"}}"#),
    ];
    bed.server.send_event(PODS, &added(&pod("web", "8")));
    for (number, line) in bad_lines.iter().enumerate() {
        let watches = lists_and_watches(&bed.server, PODS).1;
        assert_eq!(watches, number + 1);
        bed.server.send_event(PODS, line);
        in_time("the next watch", bed.server.wait_for(|_| lists_and_watches(&bed.server, PODS).1 > watches)).await;
        in_time("the open watch", bed.server.wait_for(|counters| counters.open_watches == 39)).await;
    }
    bed.server.send_event(PODS, &added(&pod("job", "11")));
    let stored = in_time("the event after the bad lines", async {
        loop {
            let snapshot = pods.read();
            if pod_names(&snapshot) == ["api", "job", "web"] {
                return snapshot;
            }
            pods.changed().await;
        }
    })
    .await;
    assert!(stored.state.is_live(), "{}", stored.state);

    let requests = bed.server.requests();
    let of_pods = of_path(&requests, PODS);
    let versions: Vec<&str> = of_pods
        .iter()
        .filter(|seen| seen.is_watch())
        .filter_map(|seen| seen.uri.query()?.split('&').find_map(|pair| pair.strip_prefix("resourceVersion=")))
        .collect();
    // The object with no name has a version, so the watch continues after it.
    assert_eq!(versions, ["7", "8", "8", "10"], "each new watch starts after the last version that the app read");
    assert_eq!(of_pods.len() - versions.len(), 1, "no new list");
    // No bad line of the pods changed the state of an other store.
    let after = stores.stores.statuses();
    for (before, after) in before.iter().zip(&after).filter(|(before, _)| before.kind != Kind::Pod) {
        assert_eq!((&after.state, after.since), (&before.state, before.since), "{:?}", after.kind);
    }
}
