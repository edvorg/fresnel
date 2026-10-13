//! The objects of the view tests: the pods and the Events of the scene on k3s that
//! `fresnel-core` keeps as its fixture. No object of a real cluster is in the repository.
//!
//! Each time of the fixture moves by one span, so that the moment of the fixture is the
//! time of the view tests: a pod then has the age that `kubectl get pods` printed.

use std::sync::{Arc, OnceLock};
use std::time::SystemTime;

use fresnel_core::app::{ClusterData, ClusterRow};
use fresnel_core::index::{Index, IndexState, ObjectEvents};
use fresnel_core::net::task::{TaskReport, Tasks};
use fresnel_core::published::{Disposal, Held, Notice, Publisher};
use fresnel_core::store::{Snapshot, StoreState};
use jiff::{SignedDuration, Timestamp};
use k8s_openapi::api::core::v1::{Event, Pod};
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::runtime::{Builder, Runtime};

use crate::fake::{SECOND, connected, live_status, now};

macro_rules! fixture {
    ($name:literal) => {
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../fresnel-core/fixtures/k3s/", $name))
    };
}

/// The moment of the fixture: `kubectl get pods` printed an age of 51 s for the pods of
/// 17:58:14.
const FIXTURE_TIME: &str = "2026-10-13T17:59:05Z";

/// Moves each time in a JSON value by `shift`.
fn move_times(value: &mut Value, shift: SignedDuration) {
    match value {
        Value::String(text) => {
            if let Ok(time) = text.parse::<Timestamp>() {
                *text = (time + shift).to_string();
            }
        }
        Value::Array(values) => values.iter_mut().for_each(|value| move_times(value, shift)),
        Value::Object(fields) => fields.values_mut().for_each(|value| move_times(value, shift)),
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn objects<T: DeserializeOwned>(text: &str) -> Vec<Arc<T>> {
    let mut value: Value = serde_json::from_str(text).expect("a file of the fixture");
    let fixture_time: Timestamp = FIXTURE_TIME.parse().expect("a time");
    let test_time = Timestamp::try_from(now()).expect("a time");
    move_times(&mut value, test_time.duration_since(fixture_time));
    let objects: Vec<T> = serde_json::from_value(value).expect("the objects of the fixture");
    objects.into_iter().map(Arc::new).collect()
}

/// The 25 pods of the scene: one for each state of the status rule.
pub(crate) fn pods() -> Vec<Arc<Pod>> {
    static PODS: OnceLock<Vec<Arc<Pod>>> = OnceLock::new();
    PODS.get_or_init(|| objects(fixture!("pods.json"))).clone()
}

/// The Events of the scene.
pub(crate) fn events() -> Vec<Arc<Event>> {
    static EVENTS: OnceLock<Vec<Arc<Event>>> = OnceLock::new();
    EVENTS.get_or_init(|| objects(fixture!("events.json"))).clone()
}

/// The pod of the scene with this name.
pub(crate) fn pod(name: &str) -> Arc<Pod> {
    let found = pods().into_iter().find(|pod| pod.metadata.name.as_deref() == Some(name));
    found.unwrap_or_else(|| panic!("the scene has the pod {name}"))
}

/// A pod after a change of its JSON: a state that the scene does not hold, or one more
/// pod for a table with many rows.
pub(crate) fn changed(pod: &Pod, change: impl FnOnce(&mut Value)) -> Arc<Pod> {
    let mut value = serde_json::to_value(pod).expect("JSON");
    change(&mut value);
    Arc::new(serde_json::from_value(value).expect("a pod"))
}

/// A copy of a pod with an other name and UID.
pub(crate) fn renamed(pod: &Pod, name: &str, uid: &str) -> Arc<Pod> {
    changed(pod, |value| {
        value["metadata"]["name"] = name.into();
        value["metadata"]["uid"] = uid.into();
    })
}

/// Many pods: copies of the pods of the scene, each with a name and a UID of its own, in
/// 20 namespaces.
pub(crate) fn many_pods(count: usize) -> Vec<Arc<Pod>> {
    let scene = pods();
    let copy = |number: usize| {
        let mut pod = Pod::clone(&scene[number % scene.len()]);
        let name = pod.metadata.name.take().unwrap_or_default();
        pod.metadata.name = Some(format!("{name}-{number:05}"));
        pod.metadata.uid = Some(format!("uid-{number:05}"));
        pod.metadata.namespace = Some(format!("team-{:02}", number / scene.len() % 20));
        Arc::new(pod)
    };
    (0..count).map(copy).collect()
}

/// The snapshot of a store after a complete list. A live store got its last data 3 s
/// before now, and each other store 48 s before now, at 07:59:12.
pub(crate) fn store<T: fresnel_core::kinds::Object>(state: StoreState, objects: &[Arc<T>]) -> Snapshot<T> {
    let last_contact = if state.is_live() { now() - 3 * SECOND } else { now() - 48 * SECOND };
    Snapshot {
        generation: 1,
        state,
        since: last_contact,
        last_contact: Some(last_contact),
        listed: true,
        dropped: 0,
        items: objects.iter().cloned().collect(),
    }
}

/// A store before its first data: no object, and no complete list.
pub(crate) fn empty_store(state: StoreState, since: SystemTime) -> Snapshot<Pod> {
    Snapshot { since, last_contact: None, listed: false, generation: 0, ..store(state, &[]) }
}

/// A value as a reader of a published state holds it. One thread frees the values of
/// each test of this program.
pub(crate) fn held<T: Send + Sync + 'static>(value: T) -> Held<T> {
    static DISPOSAL: OnceLock<(Disposal, Runtime)> = OnceLock::new();
    let (disposal, _) = DISPOSAL.get_or_init(|| {
        let runtime = Builder::new_current_thread().build().expect("a runtime");
        let tasks = Tasks::new(runtime.handle().clone(), |report: TaskReport| panic!("{report}"));
        (Disposal::start(&tasks), runtime)
    });
    Publisher::new(value, disposal.bin(), Notice::new()).read()
}

/// The same objects in an other state, as a store publishes them when its watch stops.
pub(crate) fn in_state(pods: &Snapshot<Pod>, state: StoreState) -> Snapshot<Pod> {
    Snapshot { items: pods.items.clone(), ..store(state, &[]) }
}

/// The Event index of the scene on a store of the Events in this state.
pub(crate) fn event_index(events_store: Snapshot<Event>, state: IndexState) -> Held<Index<ObjectEvents>> {
    held(Index::<ObjectEvents>::of_snapshot(&events_store, state))
}

/// The data of a cluster with this Pod store, and a live Event index of the scene.
pub(crate) fn data(pods: Snapshot<Pod>) -> ClusterData {
    let index = event_index(store(StoreState::Live, &events()), IndexState::Following);
    ClusterData { pods: held(pods), events: Some(index) }
}

/// A selected cluster with live connections and this Pod store.
pub(crate) fn cluster(name: &str, pods: Snapshot<Pod>) -> ClusterRow {
    ClusterRow { data: Some(data(pods)), ..connected(name, live_status()) }
}

/// A selected cluster with live connections, a live store and these pods.
pub(crate) fn live(name: &str, pods: &[Arc<Pod>]) -> ClusterRow {
    cluster(name, store(StoreState::Live, pods))
}
