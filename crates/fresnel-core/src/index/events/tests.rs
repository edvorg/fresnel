use k8s_openapi::api::core::v1::{Node, Pod};

use super::*;
use crate::fixture::{self, SCENE};
use crate::index::bed::{Bed, read};
use crate::index::task::Basis;
use crate::kinds::Object;

fn pod(name: &str) -> (ObjectKey, String) {
    let pod = fixture::one::<Pod>(name);
    (ObjectKey::new(SCENE, pod.meta().name.as_deref().expect("a name")), pod.meta().uid.clone().expect("a UID"))
}

fn reasons(events: &[Arc<Event>]) -> Vec<&str> {
    events.iter().map(|event| event.reason.as_deref().expect("a reason")).collect()
}

/// The names of the Events of the fixture that fit, in the sequence of the names.
fn events_where(fits: impl Fn(&Event) -> bool) -> Vec<String> {
    let mut names: Vec<String> =
        fixture::all::<Event>().into_iter().filter(fits).map(|event| event.metadata.name.expect("a name")).collect();
    names.sort_unstable();
    names
}

fn sorted_names(events: &[Arc<Event>]) -> Vec<String> {
    let mut names: Vec<String> = events.iter().map(|event| event.metadata.name.clone().expect("a name")).collect();
    names.sort_unstable();
    names
}

#[tokio::test]
async fn an_object_of_the_fixture_finds_its_events_with_the_newest_first() {
    let bed = Bed::new();
    let (index, changed) = read(&mut EventIndexer::new(bed.fixture::<Event>().reader()));
    assert!(changed);

    let (crash, uid) = pod("crash");
    let answer = index.events_of(Kind::Pod, &crash, &uid);
    assert_eq!(answer.basis, Basis::Current);
    let of_crash = events_where(|event| event.involved_object.uid.as_deref() == Some(uid.as_str()));
    assert_eq!(sorted_names(&answer.found), of_crash);
    for reason in ["Scheduled", "Pulled", "Created", "Started", "BackOff"] {
        assert!(reasons(&answer.found).contains(&reason), "{reason} is an Event of a pod that starts again and again");
    }
    let times: Vec<_> = answer.found.iter().map(|event| last_seen(event)).collect();
    assert!(times.iter().all(Option::is_some) && times.is_sorted_by(|newer, older| newer >= older), "{times:?}");

    // An other object with the same name does not get these Events: the UID decides.
    assert_eq!(index.events_of(Kind::Pod, &crash, "uid-of-a-later-pod").found.len(), 0);
    assert_eq!(index.events_of(Kind::Pod, &ObjectKey::new(SCENE, "no-such-pod"), "uid-1").found.len(), 0);
}

/// A view test builds an index with no task. It must give the answers of the task.
#[tokio::test]
async fn the_index_of_a_snapshot_gives_the_answers_of_the_task_with_the_state_of_its_caller() {
    let bed = Bed::new();
    let store = bed.fixture::<Event>();
    let (by_task, _) = read(&mut EventIndexer::new(store.reader()));
    let snapshot = store.reader().read();
    let built = Index::of_snapshot(&snapshot, IndexState::Following);
    assert_eq!((built.generation, &built.state, built.sources()), (1, &by_task.state, by_task.sources()));
    for name in ["crash", "web-", "done"] {
        let (key, uid) = pod(name);
        let (task, direct) = (by_task.events_of(Kind::Pod, &key, &uid), built.events_of(Kind::Pod, &key, &uid));
        assert!(!task.found.is_empty(), "{name} has Events");
        assert_eq!((sorted_names(&direct.found), direct.basis), (sorted_names(&task.found), Basis::Current));
    }
    let node = fixture::all::<Node>().remove(0);
    let key = ObjectKey::new("", node.meta().name.as_deref().expect("a name"));
    let uid = node.meta().uid.as_deref().expect("a UID");
    let by_name = |index: &Index<ObjectEvents>| sorted_names(&index.events_of(Kind::Node, &key, uid).found);
    assert_eq!(by_name(&built), by_name(&by_task), "the Events that name the node with no UID of the API");

    let failed = IndexState::Failed { reason: "a defect".into() };
    let stale = Index::of_snapshot(&snapshot, failed.clone());
    let (key, uid) = pod("crash");
    assert_eq!(stale.events_of(Kind::Pod, &key, &uid).basis, Basis::NotFollowed(failed));
}

/// The kubelet writes the name of its node in the place of the UID. Each other source
/// writes the UID of the Node object.
#[tokio::test]
async fn a_node_finds_the_events_that_name_it_with_no_uid_of_the_api() {
    let bed = Bed::new();
    let (index, _) = read(&mut EventIndexer::new(bed.fixture::<Event>().reader()));
    let node = fixture::all::<Node>().remove(0);
    let (name, uid) = (node.metadata.name.clone().expect("a name"), node.metadata.uid.clone().expect("a UID"));
    let about = |event: &Event, id: &str| {
        let object = &event.involved_object;
        object.kind.as_deref() == Some("Node") && object.uid.as_deref() == Some(id)
    };
    let (by_name, by_uid) = (events_where(|event| about(event, &name)), events_where(|event| about(event, &uid)));
    assert!(!by_name.is_empty() && !by_uid.is_empty(), "the fixture has the two forms: {by_name:?} {by_uid:?}");

    let found = index.events_of(Kind::Node, &ObjectKey::new("", &name), &uid).found;
    let mut expected = [by_name.clone(), by_uid].concat();
    expected.sort_unstable();
    assert_eq!(sorted_names(&found), expected);
    // A pod with the name of the node does not get the Events of the node.
    assert_eq!(index.events_of(Kind::Pod, &ObjectKey::new("", &name), "uid-1").found.len(), 0);
    assert_eq!(index.relations().by_name.keys(), 1, "only the node has Events with no UID of the API");
}

fn event(name: &str, version: &str, object: &str, count: u32) -> Event {
    let text = format!(
        r#"{{"metadata":{{"name":"{name}","namespace":"shop","uid":"uid-{name}","resourceVersion":"{version}"}},
        "involvedObject":{object},"reason":"Tested","count":{count},"lastTimestamp":"2026-01-01T00:00:{count:02}Z"}}"#
    );
    serde_json::from_str(&text).expect("an Event")
}

#[tokio::test]
async fn each_change_of_an_event_is_a_change_of_the_index_and_an_event_that_is_gone_leaves_no_entry() {
    let bed = Bed::new();
    let mut events = bed.store::<Event>();
    let mut indexer = EventIndexer::new(events.reader());
    let web = ObjectKey::new("shop", "web");
    let (index, changed) = read(&mut indexer);
    assert!(!changed);
    assert!(
        matches!(index.events_of(Kind::Pod, &web, "uid-web").basis, Basis::NotLoaded(source) if source.kind == Kind::Event)
    );

    let about_web = r#"{"kind":"Pod","namespace":"shop","name":"web","uid":"uid-web"}"#;
    events.list(vec![
        event("first", "1", about_web, 1),
        event("second", "1", about_web, 5),
        // No UID, and a kind that the app does not show: no relation.
        event("other", "1", r#"{"kind":"Widget","namespace":"shop","name":"web"}"#, 9),
        // No UID, and a kind of the app.
        event("named", "1", r#"{"kind":"Pod","namespace":"shop","name":"web"}"#, 3),
    ]);
    let (index, _) = read(&mut indexer);
    let found = |index: &Index<ObjectEvents>| {
        let answer = index.events_of(Kind::Pod, &web, "uid-web");
        assert_eq!(answer.basis, Basis::Current);
        let name = |event: &Arc<Event>| event.metadata.name.clone().expect("a name");
        answer.found.iter().map(|event| format!("{} {}", name(event), event.count.unwrap_or(0))).collect::<Vec<_>>()
    };
    assert_eq!(found(&index), ["second 5", "named 3", "first 1"], "the newest first");

    // The source repeats an Event: the same object with a new count and a new time.
    events.apply(event("first", "2", about_web, 7));
    let (repeated, changed) = read(&mut indexer);
    assert!(changed, "the answer holds the Event itself");
    assert_eq!(found(&repeated), ["first 7", "second 5", "named 3"]);
    assert_eq!(found(&index), ["second 5", "named 3", "first 1"], "an index that a reader holds does not change");

    // The Event is about an other object now, and then each Event is gone.
    events.apply(event("second", "2", r#"{"kind":"Pod","namespace":"shop","name":"db","uid":"uid-db"}"#, 5));
    let (moved, _) = read(&mut indexer);
    assert_eq!(found(&moved), ["first 7", "named 3"]);
    assert_eq!(moved.events_of(Kind::Pod, &ObjectKey::new("shop", "db"), "uid-db").found.len(), 1);
    for name in ["first", "second", "other", "named"] {
        events.delete(&event(name, "9", about_web, 1));
    }
    let (empty, changed) = read(&mut indexer);
    assert!(changed);
    let relations = empty.relations();
    assert_eq!((relations.by_uid.keys(), relations.by_name.keys(), relations.events.len()), (0, 0, 0));
}
