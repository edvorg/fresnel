use std::sync::Arc;
use std::time::{Duration, SystemTime};

use fresnel_net::WallClock;
use fresnel_net::task::{TaskReport, Tasks};
use k8s_openapi::api::core::v1::Secret;
use tokio::runtime::Handle;
use tokio::time::{Instant, sleep};

use super::*;
use crate::kinds::Meta;
use crate::published::{Disposal, Notice, Reader};
use crate::store::Wait;

const MS: Duration = Duration::from_millis(1);
const BATCH: Duration = Duration::from_millis(30);

/// A writer on the paused clock, with a reader of its snapshots.
fn writer<T: Object>() -> (Writer<T>, Reader<Snapshot<T>>, Disposal) {
    let tasks = Tasks::new(Handle::current(), |report: TaskReport| panic!("a task ended: {report}"));
    let disposal = Disposal::start(&tasks);
    let started = Instant::now();
    let clock = WallClock::new(move || SystemTime::UNIX_EPOCH + started.elapsed());
    let first = Snapshot::empty(StoreState::Waiting(Wait::Channel), clock.now());
    let publisher = Publisher::new(first, disposal.bin(), Notice::new());
    let reader = publisher.reader();
    (Writer::resume(publisher, clock, BATCH, None), reader, disposal)
}

fn object(name: &str, version: &str) -> Meta {
    let text = format!(
        r#"{{"metadata":{{"name":"{name}","namespace":"shop","uid":"uid-{name}","resourceVersion":"{version}"}}}}"#
    );
    serde_json::from_str(&text).expect("an object")
}

fn list(writer: &mut Writer<Meta>, objects: &[(&str, &str)]) {
    writer.begin_list();
    let objects = objects.iter().map(|(name, version)| object(name, version)).collect();
    writer.list_page(objects).expect("a good page");
    writer.end_list();
}

/// Publishes what waits for its batch: the time of the batch goes by.
async fn publish<T: Object>(writer: &mut Writer<T>) {
    sleep(BATCH).await;
    writer.publish_if_due();
}

fn key(name: &str) -> ObjectKey {
    ObjectKey::new("shop", name)
}

/// In the prototype, each new list was a new generation: each view and each index then
/// computed all again after a reconnect that changed nothing.
#[tokio::test(start_paused = true)]
async fn the_generation_changes_only_when_an_object_changes() {
    let (mut writer, reader, _disposal) = writer::<Meta>();
    list(&mut writer, &[("a", "1"), ("b", "1")]);
    let first = reader.read();
    assert_eq!((first.generation, first.items.len()), (1, 2));

    list(&mut writer, &[("a", "1"), ("b", "1")]);
    writer.set_state(StoreState::Live);
    writer.contact();
    publish(&mut writer).await;
    writer.delete(&object("no-such-object", "9")).expect("a name");
    publish(&mut writer).await;
    let same = reader.read();
    assert_eq!((same.generation, &same.state), (1, &StoreState::Live), "a state and a bookmark change no object");

    // A new list keeps the allocation of each object with no change.
    list(&mut writer, &[("a", "1"), ("b", "2")]);
    let second = reader.read();
    assert_eq!(second.generation, 2);
    let entry = |snapshot: &Snapshot<Meta>, name: &str| Arc::clone(snapshot.items.get(&key(name)).expect("the object"));
    assert!(Arc::ptr_eq(&entry(&first, "a"), &entry(&second, "a")));
    assert!(!Arc::ptr_eq(&entry(&first, "b"), &entry(&second, "b")));

    // A list with one object less is a change, also when each object of it is known.
    list(&mut writer, &[("a", "1")]);
    assert_eq!((reader.read().generation, reader.read().items.len()), (3, 1));
    // A new object with the name of an old one is a change: the UID differs.
    let mut reborn = object("a", "1");
    reborn.metadata.uid = Some("uid-new".to_owned());
    writer.apply(reborn).expect("a name");
    publish(&mut writer).await;
    assert_eq!(reader.read().generation, 4);
}

#[tokio::test(start_paused = true)]
async fn a_change_is_due_at_once_after_a_quiet_time_and_at_the_end_of_the_batch_after_a_snapshot() {
    let (mut writer, reader, _disposal) = writer::<Meta>();
    assert_eq!(writer.due(), None, "nothing waits");
    writer.apply(object("a", "1")).expect("a name");
    assert_eq!(writer.due(), Some(Instant::now()));
    writer.publish_if_due();
    assert_eq!((reader.read().items.len(), writer.due()), (1, None));

    sleep(10 * MS).await;
    writer.apply(object("b", "1")).expect("a name");
    writer.publish_if_due();
    assert_eq!(reader.read().items.len(), 1, "the batch of the last snapshot is not over");
    assert_eq!(writer.due(), Some(Instant::now() + 20 * MS));
    sleep(20 * MS).await;
    writer.publish_if_due();
    let snapshot = reader.read();
    assert_eq!((snapshot.items.len(), snapshot.generation), (2, 2));
    assert_eq!(snapshot.last_contact, Some(SystemTime::UNIX_EPOCH + 10 * MS), "the time of the event");
}

/// No store holds the value of a Secret (plan section 9.4): `kubectl apply` puts a copy
/// of the object into an annotation, and that copy must not stay.
#[tokio::test(start_paused = true)]
async fn an_object_goes_into_the_store_with_no_managed_fields_and_no_copy_of_kubectl_apply() {
    let text = r#"{"metadata":{"name":"db","namespace":"shop","resourceVersion":"3",
        "managedFields":[{"manager":"kubectl","operation":"Apply"}],
        "annotations":{"kubectl.kubernetes.io/last-applied-configuration":"{\"data\":{\"password\":\"c2VjcmV0\"}}"}},
        "data":{"password":"c2VjcmV0"}}"#;
    let (mut writer, reader, _disposal) = writer::<Secret>();
    writer.begin_list();
    writer.list_page(vec![serde_json::from_str(text).expect("a secret")]).expect("a good page");
    writer.end_list();
    writer.apply(serde_json::from_str(&text.replace("\"db\"", "\"api\"")).expect("a secret")).expect("a name");
    publish(&mut writer).await;
    let snapshot = reader.read();
    for name in ["db", "api"] {
        let secret = snapshot.items.get(&key(name)).expect("the secret");
        assert_eq!((&secret.metadata.managed_fields, &secret.metadata.annotations), (&None, &None), "{name}");
    }
}

/// A task that starts again continues from what the readers see (design rule 5).
#[tokio::test(start_paused = true)]
async fn a_new_writer_of_a_store_starts_with_the_objects_of_the_last_snapshot() {
    let tasks = Tasks::new(Handle::current(), |report: TaskReport| panic!("a task ended: {report}"));
    let disposal = Disposal::start(&tasks);
    let clock = WallClock::new(|| SystemTime::UNIX_EPOCH);
    let first = Snapshot::empty(StoreState::Waiting(Wait::Channel), clock.now());
    let publisher = Publisher::new(first, disposal.bin(), Notice::new());
    let mut first = Writer::resume(publisher.clone(), clock.clone(), BATCH, None);
    list(&mut first, &[("a", "1"), ("b", "1")]);
    first.set_state(StoreState::Live);
    first.begin_list();
    first.list_page(vec![object("c", "1")]).expect("a good page");
    drop(first);

    let mut second = Writer::resume(publisher.clone(), clock, BATCH, None);
    assert_eq!(second.state(), &StoreState::Live);
    second.apply(object("a", "2")).expect("a name");
    publish(&mut second).await;
    let snapshot = publisher.read();
    let names: Vec<&str> = snapshot.items.iter().map(|(key, _)| &*key.name).collect();
    assert_eq!((names, snapshot.generation), (vec!["a", "b"], 2), "half of a list is gone, and the generation goes on");
    assert!(snapshot.listed, "the list of the task before counts");
}

/// Plan section 9.8: "no pods" and "pods not loaded" differ. A store with no object tells
/// which one it is.
#[tokio::test(start_paused = true)]
async fn a_store_is_listed_after_its_first_complete_list_also_with_no_object() {
    let (mut writer, reader, _disposal) = writer::<Meta>();
    writer.begin_list();
    writer.list_page(vec![object("a", "1")]).expect("a good page");
    writer.set_state(StoreState::Listing { loaded: 1 });
    assert!(!reader.read().listed, "one page is not the list");
    writer.cancel_list();
    writer.set_state(StoreState::Waiting(Wait::Channel));
    assert!(!reader.read().listed, "a list that failed tells nothing");

    list(&mut writer, &[]);
    let empty = reader.read();
    assert_eq!((empty.listed, empty.items.len(), empty.generation), (true, 0, 0), "the cluster has no object");
    assert!(empty.status(crate::kinds::Kind::Pod).listed);
    writer.set_state(StoreState::Waiting(Wait::ListSlot));
    assert!(reader.read().listed, "a later fault does not take the objects away");
}

/// An object that the server made `second` seconds after a fixed time.
fn object_at(name: &str, version: &str, second: u8) -> Meta {
    let mut object = object(name, version);
    let time = format!("\"2026-01-01T00:00:{second:02}Z\"");
    object.metadata.creation_timestamp = Some(serde_json::from_str(&time).expect("a time"));
    object
}

fn names(snapshot: &Snapshot<Meta>) -> Vec<&str> {
    snapshot.items.iter().map(|(key, _)| &*key.name).collect()
}

/// Plan section 9.6: a maximum count applies to the Events. Design rule 12: a buffer that
/// drops data shows a counter.
#[tokio::test(start_paused = true)]
async fn a_store_with_a_limit_keeps_its_newest_objects_and_counts_each_other_one() {
    let tasks = Tasks::new(Handle::current(), |report: TaskReport| panic!("a task ended: {report}"));
    let disposal = Disposal::start(&tasks);
    let clock = WallClock::new(|| SystemTime::UNIX_EPOCH);
    let first = Snapshot::empty(StoreState::Waiting(Wait::Channel), clock.now());
    let publisher = Publisher::new(first, disposal.bin(), Notice::new());
    let mut writer = Writer::resume(publisher.clone(), clock.clone(), BATCH, Some(3));
    writer.begin_list();
    let page = [("e", 5), ("a", 1), ("d", 4), ("b", 2), ("c", 3)].map(|(name, second)| object_at(name, "1", second));
    assert_eq!(writer.list_page(page.into()).expect("a good page"), 3, "the count of the list so far");
    writer.end_list();
    let listed = publisher.read();
    assert_eq!((names(&listed), listed.dropped, listed.generation), (vec!["c", "d", "e"], 2, 1));

    // A new object goes in, and the oldest one leaves.
    writer.apply(object_at("f", "1", 6)).expect("a name");
    publish(&mut writer).await;
    let newer = publisher.read();
    assert_eq!((names(&newer), newer.dropped, newer.generation), (vec!["d", "e", "f"], 3, 2));

    // An object that is older than each object of the full store does not go in.
    writer.apply(object_at("a", "2", 1)).expect("a name");
    publish(&mut writer).await;
    let same = publisher.read();
    assert_eq!((names(&same), same.dropped, same.generation), (vec!["d", "e", "f"], 4, 2));

    // A new version of an object of the store replaces it, and no object leaves.
    writer.apply(object_at("d", "2", 4)).expect("a name");
    writer.delete(&object_at("e", "1", 5)).expect("a name");
    writer.apply(object_at("a", "2", 1)).expect("a name");
    publish(&mut writer).await;
    let replaced = publisher.read();
    assert_eq!((names(&replaced), replaced.dropped, replaced.generation), (vec!["a", "d", "f"], 4, 3));
    drop(writer);

    // A task that starts again has the limit and the count of the task before.
    let mut again = Writer::resume(publisher.clone(), clock, BATCH, Some(3));
    again.apply(object_at("g", "1", 7)).expect("a name");
    publish(&mut again).await;
    let last = publisher.read();
    assert_eq!((names(&last), last.dropped), (vec!["d", "f", "g"], 5), "the oldest object left, not the first name");
    assert_eq!(last.status(crate::kinds::Kind::Event).dropped, 5);
}
