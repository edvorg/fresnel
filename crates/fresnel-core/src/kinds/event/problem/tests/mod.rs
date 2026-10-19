//! The state of a problem on the scene of k3s: each line of the rule table with a change of
//! the object before and after the last report, an object that went away or came again
//! with an other UID, and a store that is not live.
//!
//! - `pods`: the rules of a pod, its containers and its node. `workloads`: the rules of a
//!   controller that could not make a pod, and of a Job that failed.

mod pods;
mod workloads;

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use k8s_openapi::api::apps::v1::{DaemonSet, Deployment, ReplicaSet, StatefulSet};
use k8s_openapi::api::batch::v1::{CronJob, Job};
use k8s_openapi::api::core::v1::{Namespace, Node, Pod, Service};
use k8s_openapi::api::discovery::v1::EndpointSlice;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

use super::ProblemState::{Deleted, Ongoing, Replaced, Resolved, Unknown};
use super::*;
use crate::fixture;
use crate::kinds::Meta;
use crate::net::task::Tasks;
use crate::published::{Disposal, Notice, Publisher};
use crate::search::Searched;
use crate::store::StoreState;

/// The node of the scene.
const NODE: &str = "35f62a67b541";
/// A time before each report of the scene, and one after each one.
const EARLY: &str = "2026-10-13T17:50:00Z";
const LATE: &str = "2026-10-13T18:10:00Z";
/// The moment of a judgement: 20 minutes after `LATE`.
const NOW: &str = "2026-10-13T18:30:00Z";

/// Gives the snapshots of the tests to a reader, as a store does.
struct Bed {
    disposal: Disposal,
    _runtime: Runtime,
}

impl Bed {
    fn new() -> Self {
        let runtime = Builder::new_current_thread().build().expect("a runtime");
        let tasks = Tasks::new(runtime.handle().clone(), |report| panic!("a task ended: {report}"));
        Self { disposal: Disposal::start(&tasks), _runtime: runtime }
    }

    fn source<T: Searched + Clone>(&self, kind: Kind, state: StoreState, objects: &[T]) -> Source {
        let snapshot = Snapshot {
            generation: 1,
            state,
            since: SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000),
            last_contact: None,
            listed: true,
            dropped: 0,
            items: objects.iter().cloned().map(Arc::new).collect(),
        };
        Source::of(kind, Publisher::new(snapshot, self.disposal.bin(), Notice::new()).read())
    }

    fn live<K: Describe>(&self, objects: &[K]) -> Source {
        self.source(K::ID, StoreState::Live, objects)
    }

    /// The witness of the scene with these pods and nodes, and each other kind of tier A, at
    /// `NOW`.
    fn scene(&self, pods: &[Pod], nodes: &[Node]) -> Witness {
        let sources = [
            self.live(pods),
            self.live(nodes),
            self.live(&fixture::all::<Namespace>()),
            self.live(&fixture::all::<Deployment>()),
            self.live(&fixture::all::<ReplicaSet>()),
            self.live(&fixture::all::<StatefulSet>()),
            self.live(&fixture::all::<DaemonSet>()),
            self.live(&fixture::all::<Job>()),
            self.live(&fixture::all::<CronJob>()),
            self.live(&fixture::all::<Service>()),
            self.live(&fixture::all::<EndpointSlice>()),
        ];
        Witness::new(sources, moment(NOW))
    }

    /// The scene with one object of a kind after a change of its JSON.
    fn with<K: Describe + serde::Serialize>(&self, name: &str, change: impl FnOnce(&mut Value)) -> Witness {
        let changed = fixture::changed::<K>(name, change);
        let mut witness = self.scene(&fixture::all(), &fixture::all());
        let place = witness.sources.iter().position(|source| source.status.kind == K::ID).expect("a store");
        let others = fixture::all::<K>().into_iter().filter(|object| object.metadata().name.as_deref() != Some(name));
        witness.sources[place] = self.live(&others.chain([changed]).collect::<Vec<_>>());
        witness
    }
}

/// A warning about an object of the scene, with its last report at `last`, and the part of
/// the object that it names.
fn warning<K: Describe>(name: &str, reason: &str, last: &str, part: Option<&str>) -> Event {
    let object = fixture::one::<K>(name);
    let meta = object.metadata();
    let namespace = meta.namespace.as_deref().unwrap_or_default();
    serde_json::from_value(json!({
        "metadata": {"name": format!("{name}.1"), "namespace": namespace},
        "involvedObject": {"kind": K::KIND, "namespace": namespace, "name": name, "uid": meta.uid,
            "fieldPath": part},
        "type": "Warning", "reason": reason, "lastTimestamp": last,
    }))
    .expect("an Event")
}

fn judged(witness: &Witness, event: &Event) -> (ProblemState, String) {
    let now = witness.judge(event).expect("a warning");
    (now.state, now.why)
}

fn at(time: &str) -> Option<Timestamp> {
    Some(time.parse().expect("a time"))
}

fn moment(time: &str) -> SystemTime {
    at(time).map(SystemTime::from).expect("a time")
}

fn state(state: ProblemState, why: &str) -> (ProblemState, String) {
    (state, why.to_owned())
}

#[test]
fn each_warning_of_the_scene_gets_the_state_of_its_object_and_other_events_get_none() {
    let bed = Bed::new();
    let witness = bed.scene(&fixture::all(), &fixture::all());
    let events = fixture::all::<Event>();
    let of = |name: &str, reason: &str| {
        let fits = |event: &&Event| {
            event.involved_object.name.as_deref() == Some(name) && event.reason.as_deref() == Some(reason)
        };
        judged(&witness, events.iter().find(fits).expect("an Event of the scene"))
    };
    assert_eq!(of("crash", "BackOff"), state(Ongoing, "the container crash waits: CrashLoopBackOff"));
    assert_eq!(of("init-crash", "BackOff"), state(Ongoing, "the container setup waits: CrashLoopBackOff"));
    assert_eq!(of("no-config", "Failed"), state(Ongoing, "the container main waits: CreateContainerConfigError"));
    assert_eq!(of("not-ready", "Unhealthy"), state(Ongoing, "the container main is not ready"));
    assert_eq!(of("unschedulable", "FailedScheduling"), state(Ongoing, "the pod waits for a node"));
    assert_eq!(of("fails", "BackoffLimitExceeded"), state(Ongoing, "the Job failed: BackoffLimitExceeded"));
    let sandbox = state(Resolved(at("2026-10-13T17:58:26Z")), "the pod has its sandbox");
    assert_eq!(of("crash", "FailedCreatePodSandBox"), sandbox);
    assert_eq!(of("done", "FailedCreatePodSandBox"), state(Resolved(None), "the pod ended well"));
    let failed = "the pod failed, so no container of it runs";
    assert_eq!(of("failed", "FailedCreatePodSandBox"), state(Unknown, failed));
    let rule = "the app has no rule for the reason UnexpectedJob of a CronJob";
    assert_eq!(of("tick", "UnexpectedJob"), state(Unknown, rule));
    let rule = "the app has no rule for the reason InvalidDiskCapacity of a Node";
    assert_eq!(of(NODE, "InvalidDiskCapacity"), state(Unknown, rule), "the kubelet writes no UID of the node");
    // The pods of the first ReplicaSet of web went away with its rollout.
    let deleted = state(Deleted, "the Pod is not in its store");
    assert_eq!(of("web-64cb9bd5d4-c8kcm", "FailedToRetrieveImagePullSecret"), deleted, "also with no rule");

    let mut count = [0; 5];
    for now in events.iter().filter_map(|event| witness.judge(event)) {
        count[usize::from(now.state.rank())] += 1;
    }
    assert_eq!(count, [9, 6, 12, 0, 4], "each of the 31 warnings, and no state of a Normal Event");
}

#[test]
fn an_object_that_went_away_or_came_again_has_its_fate_and_a_store_that_is_not_live_tells_nothing() {
    let bed = Bed::new();
    let event = warning::<Pod>("crash", "BackOff", "2026-10-13T17:58:53Z", Some("spec.containers{crash}"));
    let again = bed.with::<Pod>("crash", |pod| pod["metadata"]["uid"] = "a-new-uid".into());
    assert_eq!(judged(&again, &event), state(Replaced, "an other Pod has its name now"));
    let others: Vec<Pod> =
        fixture::all::<Pod>().into_iter().filter(|pod| pod.metadata.name.as_deref() != Some("crash")).collect();
    assert_eq!(judged(&bed.scene(&others, &[]), &event), state(Deleted, "the Pod is not in its store"));

    // The old store has no pod crash, and that tells nothing.
    let reason = crate::CoreError::invalid("no answer");
    let paused = StoreState::Backoff { failures: 2, reason, until: SystemTime::UNIX_EPOCH };
    let old = Witness::new([bed.source(Kind::Pod, paused.clone(), &others)], moment(NOW));
    assert_eq!(judged(&old, &event), (Unknown, format!("the store of the Pods is not live: {paused}")));
    let none = Witness::new([], moment(NOW));
    assert_eq!(judged(&none, &event), state(Unknown, "the cluster has no store of the Pods"));
}

/// A custom resource can have the name of a kind of the app in its own group: a Service of
/// Knative is not the Service of its name.
#[test]
fn an_object_of_an_other_api_group_is_not_the_object_of_its_name() {
    let bed = Bed::new();
    let witness = bed.scene(&fixture::all(), &fixture::all());
    let mut knative = warning::<Service>("web", "RevisionFailed", LATE, None);
    knative.involved_object.api_version = Some("serving.knative.dev/v1".to_owned());
    knative.involved_object.uid = Some("a-uid-of-knative".to_owned());
    let other = state(Unknown, "the app has no rule for a Service of serving.knative.dev/v1");
    assert_eq!(judged(&witness, &knative), other, "not replaced: the Service web is an other object");
    knative.involved_object.api_version = Some("v1".to_owned());
    assert_eq!(judged(&witness, &knative).0, Replaced, "a Service of the core group");

    let mut created = warning::<ReplicaSet>("web-6888b778b6", "FailedCreate", LATE, None);
    created.involved_object.api_version = Some("apps/v1".to_owned());
    assert_eq!(judged(&witness, &created), state(Resolved(None), "the ReplicaSet has 2 of 2 pods"));
}

#[test]
fn the_witness_keeps_the_first_full_store_of_a_kind_and_knows_the_same_snapshots_and_moment() {
    let bed = Bed::new();
    let pods = bed.live(&fixture::all::<Pod>());
    let metas: Vec<Meta> = fixture::all::<Node>().iter().map(|node| Meta { metadata: node.metadata.clone() }).collect();
    let events = bed.live(&fixture::all::<Event>());
    let sources = [pods.clone(), bed.live::<Pod>(&[]), bed.source(Kind::Node, StoreState::Live, &metas), events];
    let witness = Witness::new(sources, moment(NOW));
    let kinds = witness.sources.iter().map(|source| source.status.kind).collect::<Vec<_>>();
    assert_eq!(kinds, [Kind::Pod], "no metadata, and no Events: they judge nothing");
    assert!(witness.store::<Pod>().is_ok_and(|pods| pods.items.len() == 25), "the first store of the Pods");
    assert!(witness.is_same(&Witness::new([pods.clone()], moment(NOW))), "one snapshot");
    assert!(!witness.is_same(&Witness::new([pods.clone()], moment(LATE))), "an other moment");
    assert!(!witness.is_same(&Witness::new([bed.live(&fixture::all::<Pod>())], moment(NOW))), "a new snapshot");
    let more = Witness::new([pods, bed.live(&fixture::all::<Job>())], moment(NOW));
    assert!(!witness.is_same(&more), "one more store");
}
