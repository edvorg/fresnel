//! The indexes of one cluster on the fake API server: an event of the server reaches an
//! answer of an index, and a store that the server refuses shows in the answer.

use std::time::Duration;

use fresnel_core::index::{Answer, Basis, ClusterIndexes, IndexState, ObjectRef, read_events};
use fresnel_core::kinds::Kind;
use fresnel_core::net::{ErrorClass, NetSettings};
use fresnel_core::store::{EventsMode, ObjectKey, StoreSettings, StoreState, WatchSettings};
use fresnel_testkit::Fault;
use tokio::time::timeout;

use crate::bed::Bed;
use crate::common::LIMIT;
use crate::store_bed::Stores;
use crate::stores::PODS;

const REPLICA_SETS: &str = "/apis/apps/v1/replicasets";
const EVENTS: &str = "/api/v1/events";

/// One object on one line, as a watch sends it. Each object has one owner.
fn object(name: &str, uid: &str, owner: &str, rest: &str) -> String {
    let owners = format!(r#"[{{"apiVersion":"v1","kind":"Owner","name":"owner","uid":"{owner}"}}]"#);
    let identity = format!(r#""name":"{name}","namespace":"shop","uid":"{uid}","resourceVersion":"5""#);
    format!(r#"{{"metadata":{{{identity},"ownerReferences":{owners}}}{rest}}}"#)
}

fn pod(name: &str, owner: &str) -> String {
    let volume = r#","spec":{"containers":[],"volumes":[{"name":"config","configMap":{"name":"settings"}}]}"#;
    object(name, &format!("uid-{name}"), owner, volume)
}

fn list(items: &[String]) -> String {
    format!(r#"{{"metadata":{{"resourceVersion":"7"}},"items":[{}]}}"#, items.join(","))
}

/// Waits until an answer of an index of the cluster meets the condition.
async fn until<T>(indexes: &ClusterIndexes, what: &str, answer: impl Fn() -> Option<T>) -> T {
    let mut changes = indexes.changes();
    let wait = async {
        loop {
            if let Some(answer) = answer() {
                return answer;
            }
            changes.next().await;
        }
    };
    timeout(LIMIT, wait).await.unwrap_or_else(|_| panic!("in time: {what}"))
}

fn names(found: &[ObjectRef]) -> Vec<&str> {
    found.iter().map(|object| &*object.key.name).collect()
}

#[tokio::test]
async fn an_event_of_the_server_reaches_each_index_and_a_stopped_index_keeps_its_answers() {
    let bed = Bed::start(NetSettings::default()).await;
    bed.server.set_body(REPLICA_SETS, &list(&[object("web-1", "set-1", "deployment-1", "")]));
    bed.server.set_body(PODS, &list(&[pod("web-1-a", "set-1"), pod("web-1-b", "set-1")]));
    let about =
        r#","involvedObject":{"kind":"Pod","namespace":"shop","name":"web-1-a","uid":"uid-web-1-a"},"type":"Warning""#;
    bed.server.set_body(EVENTS, &list(&[object("web-1-a.1", "event-1", "none", about)]));
    let stores = Stores::start(&bed.tasks, &bed.link, StoreSettings::default());
    let indexes = stores.indexes(&bed.tasks);
    let (owners, references) = (indexes.owners(), indexes.references());
    let events = indexes.events().expect("the Events have a store");

    let of_deployment = || owners.read().pods_of(Kind::Deployment, "deployment-1");
    let current = |answer: Answer<Vec<ObjectRef>>| (answer.basis == Basis::Current).then_some(answer.found);
    let pods = until(&indexes, "each store behind the answer is live", || current(of_deployment())).await;
    assert_eq!(names(&pods), ["web-1-a", "web-1-b"]);
    assert_eq!(names(&owners.read().owned(Kind::Deployment, "deployment-1").found), ["web-1"]);
    let settings = ObjectKey::new("shop", "settings");
    let users = until(&indexes, "the reference index", || {
        Some(references.read().used_by(Kind::ConfigMap, &settings)).filter(|answer| answer.basis == Basis::Current)
    });
    assert_eq!(users.await.found.len(), 2);
    let of_pod = || events.read().events_of(Kind::Pod, &ObjectKey::new("shop", "web-1-a"), "uid-web-1-a");
    let warnings = until(&indexes, "the Event index", || Some(of_pod()).filter(|answer| !answer.found.is_empty()));
    assert_eq!(warnings.await.found[0].metadata.name.as_deref(), Some("web-1-a.1"));

    // One event of the watch: the store, then the index, then the answer of a reader.
    let before = owners.read();
    bed.server.send_event(PODS, &format!(r#"{{"type":"ADDED","object":{}}}"#, pod("web-1-c", "set-1")));
    let three = until(&indexes, "the pod of the event", || Some(of_deployment().found).filter(|pods| pods.len() == 3));
    assert_eq!(names(&three.await), ["web-1-a", "web-1-b", "web-1-c"]);
    assert_eq!(before.pods_of(Kind::Deployment, "deployment-1").found.len(), 2, "an index that a reader holds");
    assert!(owners.read().generation > before.generation);

    let tasks = || bed.tasks.tasks.snapshot().iter().map(|task| task.name.to_string()).collect::<Vec<_>>();
    for index in ["owner index [bed]", "reference index [bed]", "event index [bed]"] {
        assert!(tasks().contains(&index.to_owned()), "{index} is a task with a state: {:?}", tasks());
    }
    drop(indexes);
    let stopped = owners.read();
    assert_eq!(stopped.state, IndexState::Stopped);
    let last = stopped.pods_of(Kind::Deployment, "deployment-1");
    assert_eq!((last.found.len(), last.basis), (3, Basis::NotFollowed(IndexState::Stopped)));
    assert!(tasks().iter().all(|task| !task.contains("index")), "{:?}", tasks());
}

/// Plan section 9.8 and design rule 5: an answer with no pod tells if the cluster has no
/// pod or if the app could not read the pods.
#[tokio::test]
async fn pods_that_the_server_refuses_are_not_loaded_and_events_on_demand_have_no_index() {
    let bed = Bed::start(NetSettings::default()).await;
    bed.server.set_body(REPLICA_SETS, &list(&[object("web-1", "set-1", "deployment-1", "")]));
    bed.server.add_fault(Fault::forbidden().on_path(PODS));
    // The store of a kind that the server refuses asks again after this time.
    let watch = WatchSettings { forbidden_retry: Duration::from_millis(200), ..WatchSettings::default() };
    let settings = StoreSettings { watch, events: EventsMode::OnDemand, ..StoreSettings::default() };
    let stores = Stores::start(&bed.tasks, &bed.link, settings);
    let indexes = stores.indexes(&bed.tasks);
    assert!(indexes.events().is_none(), "no store, so no index: a view reads the Events of its object");
    the_events_of_one_object_come_from_the_server(&bed).await;
    let owners = indexes.owners();

    let refused = |basis: &Basis| matches!(basis, Basis::NotLoaded(source) if matches!(source.state, StoreState::Forbidden { .. }));
    let of_deployment = || owners.read().pods_of(Kind::Deployment, "deployment-1");
    let answer = until(&indexes, "the refusal", || Some(of_deployment()).filter(|answer| refused(&answer.basis))).await;
    let Basis::NotLoaded(source) = &answer.basis else { panic!("not loaded") };
    assert_eq!((answer.found.len(), source.kind, source.listed), (0, Kind::Pod, false));
    assert!(!answer.basis.is_loaded());
    // The ReplicaSets of the Deployment are a fact, and so is "no Job" for a CronJob.
    let sets = until(&indexes, "the ReplicaSets", || {
        Some(owners.read().owned(Kind::Deployment, "deployment-1")).filter(|answer| answer.basis == Basis::Current)
    });
    assert_eq!(names(&sets.await.found), ["web-1"]);
    let jobs = until(&indexes, "the Jobs", || {
        Some(owners.read().owned(Kind::CronJob, "cron-1")).filter(|answer| answer.basis == Basis::Current)
    });
    assert_eq!(jobs.await.found, [], "the cluster has no Job");

    // The server takes the list again: the same answer is now "no pods".
    bed.server.clear_faults();
    let loaded = || Some(of_deployment()).filter(|answer| answer.basis == Basis::Current);
    let none = until(&indexes, "the list of the pods", loaded).await;
    assert_eq!((none.found.len(), none.basis.is_loaded()), (0, true));
}

/// With no store of the Events, a view reads the Events of its object. The server selects
/// by kind and name, and the app by the UID: an earlier object of that name had other Events.
async fn the_events_of_one_object_come_from_the_server(bed: &Bed) {
    let event = |name: &str, uid: &str, second: u8| {
        let about = format!(r#""involvedObject":{{"kind":"Pod","namespace":"shop","name":"web","uid":"{uid}"}}"#);
        format!(
            r#"{{"metadata":{{"name":"{name}","namespace":"shop"}},{about},"lastTimestamp":"2026-01-01T00:00:{second:02}Z"}}"#
        )
    };
    let events = [event("old", "uid-web", 1), event("new", "uid-web", 9), event("earlier-pod", "uid-before", 5)];
    bed.server.set_body("/api/v1/namespaces/shop/events", &list(&events));
    let web = ObjectKey::new("shop", "web");
    let read = read_events(&bed.link.gateway, Kind::Pod, &web, "uid-web", 10).await.expect("the Events of the pod");
    let names: Vec<_> = read.events.iter().map(|event| event.metadata.name.as_deref().expect("a name")).collect();
    assert_eq!((names, read.more), (vec!["new", "old"], false), "the newest first");
    let requests = bed.server.requests();
    let asked = requests.iter().find(|seen| seen.uri.path() == "/api/v1/namespaces/shop/events").expect("the request");
    let selector = "fieldSelector=involvedObject.kind%3DPod%2CinvolvedObject.name%3Dweb";
    assert_eq!(asked.uri.query(), Some(format!("&{selector}&limit=500").as_str()));

    let one = read_events(&bed.link.gateway, Kind::Pod, &web, "uid-web", 1).await.expect("the Events of the pod");
    assert_eq!((one.events.len(), one.more), (1, true), "the read tells that the server has more");
    bed.server.add_fault(Fault::forbidden().on_path("/api/v1/namespaces/shop/events"));
    let refused = read_events(&bed.link.gateway, Kind::Pod, &web, "uid-web", 10).await.expect_err("a refusal");
    assert_eq!(refused.status(), Some(403), "{refused}");
}

/// A stop in the other sequence: the stores of a cluster stop, and its indexes live on.
#[tokio::test]
async fn an_index_of_stores_that_stopped_keeps_its_answers_and_tells_that_they_are_old() {
    let bed = Bed::start(NetSettings::default()).await;
    bed.server.set_body(REPLICA_SETS, &list(&[object("web-1", "set-1", "deployment-1", "")]));
    bed.server.set_body(PODS, &list(&[pod("web-1-a", "set-1")]));
    let stores = Stores::start(&bed.tasks, &bed.link, StoreSettings::default());
    let indexes = stores.indexes(&bed.tasks);
    let owners = indexes.owners();
    let of_deployment = || owners.read().pods_of(Kind::Deployment, "deployment-1");
    let current = || Some(of_deployment()).filter(|answer| answer.basis == Basis::Current);
    assert_eq!(names(&until(&indexes, "each store behind the answer is live", current).await.found), ["web-1-a"]);

    drop(stores);
    let stopped = |basis: &Basis| matches!(basis, Basis::Old(source) if source.state == StoreState::Stopped);
    let old = || Some(of_deployment()).filter(|answer| stopped(&answer.basis));
    assert_eq!(names(&until(&indexes, "the stop of the stores", old).await.found), ["web-1-a"]);
    // The task of the index has no store to follow now. It waits: that is no fault, and
    // the index gets no new value.
    let mut later = indexes.owners();
    assert!(timeout(Duration::from_millis(300), later.changed()).await.is_err(), "an index with no change");
    assert_eq!(owners.read().state, IndexState::Following);
    drop(indexes);
    assert_eq!(owners.read().state, IndexState::Stopped);
}

/// Bad data of a server: a page of the Events that names the token of its own request.
/// A read that follows that token asks for the same page again and again, with no pause.
#[tokio::test]
async fn a_read_of_the_events_ends_when_the_server_gives_the_token_of_a_page_two_times() {
    let bed = Bed::start(NetSettings::default()).await;
    let path = "/api/v1/namespaces/shop/events";
    bed.server.set_body(path, r#"{"metadata":{"resourceVersion":"7","continue":"again"},"items":[]}"#);
    let web = ObjectKey::new("shop", "web");
    let read = timeout(LIMIT, read_events(&bed.link.gateway, Kind::Pod, &web, "uid-web", 10));
    let refused = read.await.expect("the read ends").expect_err("a list with no end");
    assert_eq!(refused.class(), ErrorClass::Invalid, "{refused}");
    let pages = bed.server.requests().iter().filter(|seen| seen.uri.path() == path).count();
    assert_eq!(pages, 2, "the first page, and the page of its token");
}
