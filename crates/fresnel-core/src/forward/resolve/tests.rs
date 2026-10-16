//! The target of each kind on the scene of the fixture: two ready pods of the Deployment
//! `web` behind the Services `web` and `public`, the StatefulSet `db`, the DaemonSet `agent`,
//! and pods in each state of the status rule.

use std::time::SystemTime;

use k8s_openapi::api::apps::v1::{DaemonSet, Deployment, ReplicaSet, StatefulSet};
use k8s_openapi::api::batch::v1::Job;
use k8s_openapi::api::core::v1::{Pod, Service};
use k8s_openapi::api::discovery::v1::EndpointSlice;
use serde_json::{Value, json};

use super::*;
use crate::cluster::ClusterId;
use crate::fixture::{self, SCENE};
use crate::forward::rule::{Bind, LocalPort};
use crate::index::IndexState;
use crate::kinds::{Describe, Object};
use crate::store::StoreState;

/// A live store after a complete list.
fn store<T: Object>(objects: Vec<T>) -> Snapshot<T> {
    Snapshot {
        generation: 1,
        state: StoreState::Live,
        since: SystemTime::UNIX_EPOCH,
        last_contact: Some(SystemTime::UNIX_EPOCH),
        listed: true,
        dropped: 0,
        items: objects.into_iter().map(Arc::new).collect(),
    }
}

/// The stores of the scene, with its pods and slices or with changed ones.
struct Scene {
    pods: Snapshot<Pod>,
    services: Snapshot<Service>,
    owners: Index<Owners>,
}

impl Scene {
    fn of(pods: Vec<Pod>, slices: Vec<EndpointSlice>) -> Self {
        let pods = store(pods);
        let (replica_sets, jobs) = (store(fixture::all::<ReplicaSet>()), store(fixture::all::<Job>()));
        let owners = Index::<Owners>::of_snapshots(&replica_sets, &jobs, &pods, &store(slices), IndexState::Following);
        Self { pods, services: store(fixture::all()), owners }
    }

    fn fixture() -> Self {
        Self::of(fixture::all(), fixture::all())
    }

    /// The target of a rule of this kind for the object with this name and port.
    fn resolve<K: Describe>(&self, name: &str, port: u16) -> Resolution {
        self.resolve_with::<K>(name, port, None, &[])
    }

    fn resolve_with<K: Describe>(
        &self,
        name: &str,
        port: u16,
        keep: Option<&TargetPod>,
        avoid: &[Arc<str>],
    ) -> Resolution {
        let owner = match K::ID {
            Kind::Pod | Kind::Service => Owner::NotLoaded,
            _ => fixture::all::<K>()
                .iter()
                .find(|object| object.meta().name.as_deref() == Some(name))
                .and_then(|object| object.meta().uid.as_deref())
                .map_or(Owner::Gone, |uid| Owner::Found(uid.into())),
        };
        let inputs = Inputs { pods: &self.pods, services: &self.services, owners: &self.owners, owner };
        resolve(&rule(K::ID, name, port), &inputs, keep, avoid)
    }
}

fn rule(kind: Kind, name: &str, port: u16) -> ForwardRule {
    ForwardRule {
        cluster: ClusterId::new("ams"),
        kind,
        key: ObjectKey::new(SCENE, name),
        port,
        local: LocalPort::Automatic,
        bind: Bind::Localhost,
    }
}

/// The name of the pod of a resolution, with its port and a mark for a pod that only runs.
fn chosen(resolution: &Resolution) -> String {
    match resolution {
        Resolution::Pod(target) => {
            let mark = if target.ready { "" } else { ", not ready" };
            format!("{}:{}{mark}", target.pod.name, target.port)
        }
        Resolution::NoPod(reason) => format!("no pod: {reason}"),
        Resolution::NotKnown(reason) => format!("not known: {reason}"),
    }
}

fn target(resolution: Resolution) -> TargetPod {
    match resolution {
        Resolution::Pod(target) => target,
        other => panic!("a pod: {other:?}"),
    }
}

const WEB_A: &str = "web-6888b778b6-75md7";
const WEB_B: &str = "web-6888b778b6-p2twl";

#[test]
fn a_pod_target_must_run_and_may_be_not_ready() {
    let scene = Scene::fixture();
    assert_eq!(chosen(&scene.resolve::<Pod>(WEB_A, 8080)), format!("{WEB_A}:8080"), "any port of the pod");
    assert_eq!(chosen(&scene.resolve::<Pod>("not-ready", 80)), "not-ready:80, not ready");
    let no_pod = [
        ("gated", "the Pod scene/gated does not run: its phase is Pending"),
        ("done", "the Pod scene/done does not run: its phase is Succeeded"),
        ("leaving", "the Pod scene/leaving ends"),
        ("gone", "the Pod scene/gone does not exist"),
    ];
    for (name, reason) in no_pod {
        assert_eq!(scene.resolve::<Pod>(name, 80), Resolution::NoPod(reason.to_owned()), "{name}");
    }
}

#[test]
fn a_service_takes_a_ready_pod_of_its_endpoint_slices_and_the_port_of_its_target_port() {
    let scene = Scene::fixture();
    // `web` has the target port "http", the name of port 80 of the container.
    let web = target(scene.resolve::<Service>("web", 80));
    assert!([WEB_A, WEB_B].contains(&&*web.pod.name) && web.port == 80 && web.ready, "{web:?}");
    // `mixed` has a pod that ends, one that is not ready and one that is ready.
    assert_eq!(chosen(&scene.resolve::<Service>("mixed", 80)), "sidecar:80");
    assert_eq!(chosen(&scene.resolve::<Service>("db", 5432)).split('-').next(), Some("db"));
    let no_pod = [
        ("nobody", 80, "the Service scene/nobody has no pod in its EndpointSlices"),
        ("db", 9999, "the Service scene/db has no port 9999"),
        ("gone", 80, "the Service scene/gone does not exist"),
    ];
    for (name, port, reason) in no_pod {
        assert_eq!(scene.resolve::<Service>(name, port), Resolution::NoPod(reason.to_owned()), "{name}");
    }
}

/// With no ready pod, a Service takes a pod that runs, and the view marks it.
#[test]
fn a_service_with_no_ready_pod_takes_one_that_runs() {
    let slices: Vec<EndpointSlice> = fixture::all();
    let without_sidecar = |slice: EndpointSlice| {
        let mut value = serde_json::to_value(slice).expect("JSON");
        if let Some(endpoints) = value["endpoints"].as_array_mut() {
            endpoints.retain(|endpoint| endpoint["targetRef"]["name"] != "sidecar");
        }
        serde_json::from_value(value).expect("a slice")
    };
    let scene = Scene::of(fixture::all(), slices.into_iter().map(without_sidecar).collect());
    assert_eq!(chosen(&scene.resolve::<Service>("mixed", 80)), "not-ready:80, not ready");
}

#[test]
fn a_service_port_of_udp_is_no_target() {
    let mut scene = Scene::fixture();
    let udp: Service = fixture::changed("db", |value: &mut Value| {
        value["spec"]["ports"] = json!([{"port": 53, "protocol": "UDP", "targetPort": 53}]);
    });
    scene.services = store(vec![udp]);
    let reason = "port 53 of the Service scene/db is UDP, and a forward carries TCP";
    assert_eq!(scene.resolve::<Service>("db", 53), Resolution::NoPod(reason.to_owned()));
}

#[test]
fn each_workload_takes_a_ready_pod_of_its_own() {
    let scene = Scene::fixture();
    let web = target(scene.resolve::<Deployment>("web", 80));
    assert!([WEB_A, WEB_B].contains(&&*web.pod.name), "{web:?}");
    assert_eq!(target(scene.resolve::<ReplicaSet>("web-6888b778b6", 80)).pod.name, web.pod.name);
    assert!(chosen(&scene.resolve::<StatefulSet>("db", 5432)).starts_with("db-"));
    assert!(chosen(&scene.resolve::<DaemonSet>("agent", 9100)).starts_with("agent-"));
    let none = |name: &str, kind: &str| format!("no pod: the {kind} scene/{name} has no pod that runs");
    assert_eq!(chosen(&scene.resolve::<Deployment>("broken", 80)), none("broken", "Deployment"));
    assert_eq!(chosen(&scene.resolve::<Job>("fails", 80)), none("fails", "Job"));
    assert_eq!(chosen(&scene.resolve::<Deployment>("gone", 80)), "no pod: the Deployment scene/gone does not exist");
}

/// A ReplicationController has a store of the metadata only, which gives its UID, and its
/// pods name it as their owner.
#[test]
fn a_replication_controller_takes_the_pods_that_name_it_as_their_owner() {
    let owned: Pod = fixture::changed("sidecar", |value: &mut Value| {
        value["metadata"]["ownerReferences"] = json!([{"apiVersion": "v1", "kind": "ReplicationController",
            "name": "legacy", "uid": "rc-1", "controller": true}]);
    });
    let mut pods: Vec<Pod> = fixture::all();
    pods.retain(|pod| pod.metadata.name.as_deref() != Some("sidecar"));
    pods.push(owned);
    let scene = Scene::of(pods, fixture::all());
    let inputs = Inputs {
        pods: &scene.pods,
        services: &scene.services,
        owners: &scene.owners,
        owner: Owner::Found("rc-1".into()),
    };
    assert_eq!(chosen(&resolve(&rule(Kind::ReplicationController, "legacy", 80), &inputs, None, &[])), "sidecar:80");
    let loading = Inputs { owner: Owner::NotLoaded, ..inputs };
    let text = "the ReplicationControllers are not loaded yet".to_owned();
    assert_eq!(
        resolve(&rule(Kind::ReplicationController, "legacy", 80), &loading, None, &[]),
        Resolution::NotKnown(text)
    );
}

#[test]
fn the_pod_of_now_stays_while_it_fits_and_a_pod_that_failed_its_open_waits() {
    let scene = Scene::fixture();
    let first = target(scene.resolve::<Deployment>("web", 80));
    let other = if &*first.pod.name == WEB_A { WEB_B } else { WEB_A };
    let kept = ForwardRule { ..rule(Kind::Deployment, "web", 80) };
    let inputs = |owner| Inputs { pods: &scene.pods, services: &scene.services, owners: &scene.owners, owner };
    let uid: Arc<str> = fixture::one::<Deployment>("web").metadata.uid.as_deref().expect("a UID").into();
    let found = || Owner::Found(Arc::clone(&uid));
    let keep_other = TargetPod { pod: ObjectKey::new(SCENE, other), ..first.clone() };
    let other_uid = target(scene.resolve::<Pod>(other, 80)).uid;
    let keep_other = TargetPod { uid: Arc::clone(&other_uid), ..keep_other };
    assert_eq!(target(resolve(&kept, &inputs(found()), Some(&keep_other), &[])).pod.name.as_ref(), other, "kept");
    let avoided = resolve(&kept, &inputs(found()), Some(&keep_other), &[other_uid]);
    assert_eq!(target(avoided).pod.name, first.pod.name, "a pod that the server did not find waits");

    // A pod that only runs gives way to a ready one.
    let slow = TargetPod { pod: ObjectKey::new(SCENE, "not-ready"), uid: "x".into(), port: 80, ready: false };
    assert_eq!(target(resolve(&kept, &inputs(found()), Some(&slow), &[])).pod.name, first.pod.name);
}

/// A pod of now that is no longer ready stays while no other pod is ready: a forward does
/// not move to an other pod that is not ready either.
#[test]
fn the_pod_of_now_stays_when_it_and_each_other_pod_are_not_ready() {
    let not_ready = |name: &str| {
        fixture::changed::<Pod>(name, |value: &mut Value| fixture::condition(value, "Ready")["status"] = json!("False"))
    };
    let mut pods: Vec<Pod> = fixture::all();
    pods.retain(|pod| ![WEB_A, WEB_B].contains(&pod.metadata.name.as_deref().unwrap_or_default()));
    pods.extend([not_ready(WEB_A), not_ready(WEB_B)]);
    let scene = Scene::of(pods, fixture::all());
    let was_ready = TargetPod { ready: true, ..target(Scene::fixture().resolve::<Pod>(WEB_B, 80)) };
    let kept = target(scene.resolve_with::<Deployment>("web", 80, Some(&was_ready), &[]));
    assert_eq!((&*kept.pod.name, kept.ready), (WEB_B, false));
}

#[test]
fn a_store_with_no_complete_list_gives_no_answer() {
    let mut scene = Scene::fixture();
    scene.services.listed = false;
    assert_eq!(scene.resolve::<Service>("web", 80), Resolution::NotKnown("the Services are not loaded yet".to_owned()));
    scene.pods.listed = false;
    assert_eq!(scene.resolve::<Pod>(WEB_A, 80), Resolution::NotKnown("the pods are not loaded yet".to_owned()));
}
