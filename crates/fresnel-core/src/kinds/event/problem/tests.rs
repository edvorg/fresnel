//! The state of a problem on the scene of k3s: each line of the rule table with a change of
//! the object before and after the last report, an object that went away or came again
//! with an other UID, and a store that is not live.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use k8s_openapi::api::apps::v1::{DaemonSet, Deployment, ReplicaSet, StatefulSet};
use k8s_openapi::api::batch::v1::{CronJob, Job};
use k8s_openapi::api::core::v1::{Namespace, Node, Pod, Service};
use k8s_openapi::api::discovery::v1::EndpointSlice;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

use super::ProblemState::{Gone, Ongoing, Resolved, Unknown};
use super::*;
use crate::fixture::{self, condition};
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

    /// The witness of the scene with these pods and nodes, and each other kind of tier A.
    fn scene(&self, pods: &[Pod], nodes: &[Node]) -> Witness {
        Witness::new([
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
        ])
    }
}

/// The scene with one pod after a change of its JSON.
fn with_pod(bed: &Bed, name: &str, change: impl FnOnce(&mut Value)) -> Witness {
    let pods = fixture::all::<Pod>().into_iter().filter(|pod| pod.metadata.name.as_deref() != Some(name));
    let pods: Vec<Pod> = pods.chain([fixture::changed::<Pod>(name, change)]).collect();
    bed.scene(&pods, &fixture::all())
}

/// The scene with its node after a change of its JSON.
fn with_node(bed: &Bed, change: impl FnOnce(&mut Value)) -> Witness {
    bed.scene(&fixture::all(), &[fixture::changed::<Node>(NODE, change)])
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
    assert_eq!(of("not-ready", "Unhealthy"), state(Ongoing, "the pod is not ready"));
    assert_eq!(of("unschedulable", "FailedScheduling"), state(Ongoing, "the pod waits for a node"));
    assert_eq!(of("fails", "BackoffLimitExceeded"), state(Ongoing, "the Job is not healthy: failed"));
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
    let gone = state(Gone, "the Pod is not in its store");
    assert_eq!(of("web-64cb9bd5d4-c8kcm", "FailedToRetrieveImagePullSecret"), gone, "also with no rule");

    let mut count = [0; 4];
    for now in events.iter().filter_map(|event| witness.judge(event)) {
        count[match now.state {
            Ongoing => 0,
            Resolved(_) => 1,
            Unknown => 2,
            Gone => 3,
        }] += 1;
    }
    assert_eq!(count, [9, 12, 6, 4], "each of the 31 warnings, and no state of a Normal Event");
}

#[test]
fn a_failed_probe_ends_when_the_pod_is_ready_after_the_last_report() {
    let bed = Bed::new();
    let event = warning::<Pod>("not-ready", "Unhealthy", "2026-10-13T17:58:50Z", Some("spec.containers{main}"));
    let ready = |since: Option<&str>| {
        with_pod(&bed, "not-ready", |pod| {
            let ready = condition(pod, "Ready");
            ready["status"] = "True".into();
            ready["lastTransitionTime"] = since.map_or(Value::Null, Value::from);
        })
    };
    let resolved = state(Resolved(at(LATE)), "the pod is ready");
    assert_eq!(judged(&ready(Some(LATE)), &event), resolved);
    let before = state(Unknown, "the pod is ready since before the last report");
    assert_eq!(judged(&ready(Some(EARLY)), &event), before, "a failure under the threshold of the probe");
    let no_time = state(Unknown, "the pod is ready, and its status gives no time of the change");
    assert_eq!(judged(&ready(None), &event), no_time);
}

#[test]
fn a_container_problem_ends_with_a_run_or_a_good_end_after_the_last_report() {
    let bed = Bed::new();
    let event = warning::<Pod>("crash", "BackOff", "2026-10-13T17:58:53Z", Some("spec.containers{crash}"));
    let in_state = |state: Value| with_pod(&bed, "crash", |pod| pod["status"]["containerStatuses"][0]["state"] = state);
    let runs = |since: &str| in_state(json!({"running": {"startedAt": since}}));
    assert_eq!(judged(&runs(LATE), &event), state(Resolved(at(LATE)), "the container crash runs"));
    let before = state(Unknown, "the container crash runs since before the last report");
    assert_eq!(judged(&runs(EARLY), &event), before);
    let ended = |code: i32| in_state(json!({"terminated": {"exitCode": code, "startedAt": LATE, "finishedAt": LATE}}));
    assert_eq!(judged(&ended(0), &event), state(Resolved(at(LATE)), "the container crash ended well"));
    assert_eq!(judged(&ended(1), &event), state(Ongoing, "the container crash ended with exit code 1"));

    let witness = bed.scene(&fixture::all(), &fixture::all());
    let no_part = warning::<Pod>("crash", "BackOff", "2026-10-13T17:58:53Z", None);
    assert_eq!(judged(&witness, &no_part), state(Unknown, "the Event names no container"));
    let other = warning::<Pod>("crash", "Failed", "2026-10-13T17:58:53Z", Some("spec.containers{other}"));
    assert_eq!(judged(&witness, &other), state(Unknown, "the pod gives no state of the container other"));
}

#[test]
fn a_pod_with_no_node_or_no_sandbox_gets_one_after_the_last_report() {
    let bed = Bed::new();
    let scheduled = warning::<Pod>("unschedulable", "FailedScheduling", "2026-10-13T17:58:14Z", None);
    let placed = with_pod(&bed, "unschedulable", |pod| {
        let scheduled = condition(pod, "PodScheduled");
        (scheduled["status"], scheduled["lastTransitionTime"]) = ("True".into(), LATE.into());
    });
    assert_eq!(judged(&placed, &scheduled), state(Resolved(at(LATE)), "the pod has a node"));

    let sandbox = warning::<Pod>("crash", "FailedCreatePodSandBox", LATE, None);
    let witness = bed.scene(&fixture::all(), &fixture::all());
    let before = state(Unknown, "the pod has its sandbox since before the last report");
    assert_eq!(judged(&witness, &sandbox), before);
    let none = with_pod(&bed, "crash", |pod| {
        let conditions = pod["status"]["conditions"].as_array_mut().expect("conditions");
        conditions.retain(|condition| condition["type"] != "PodReadyToStartContainers");
    });
    let old_cluster = state(Unknown, "the pod has no condition PodReadyToStartContainers");
    assert_eq!(judged(&none, &sandbox), old_cluster, "a cluster older than Kubernetes 1.29");
    let waits = with_pod(&bed, "crash", |pod| condition(pod, "PodReadyToStartContainers")["status"] = "False".into());
    assert_eq!(judged(&waits, &sandbox), state(Ongoing, "the pod has no sandbox"));
}

#[test]
fn a_node_that_stopped_its_reports_or_had_a_shortage_is_judged_by_its_conditions() {
    let bed = Bed::new();
    let lost = warning::<Pod>("db-0", "NodeNotReady", "2026-10-13T17:59:00Z", None);
    let ready = |status: &str, since: &str| {
        with_node(&bed, |node| {
            let ready = condition(node, "Ready");
            (ready["status"], ready["lastTransitionTime"]) = (status.into(), since.into());
        })
    };
    let node = format!("the node {NODE}");
    assert_eq!(judged(&ready("Unknown", LATE), &lost), (Ongoing, format!("{node} is not ready")));
    assert_eq!(judged(&ready("True", LATE), &lost), (Resolved(at(LATE)), format!("{node} is ready")));
    let before = (Unknown, format!("{node} is ready since before the last report"));
    assert_eq!(judged(&ready("True", EARLY), &lost), before);
    let no_node = bed.scene(&fixture::all(), &[]);
    assert_eq!(judged(&no_node, &lost), (Unknown, format!("{node} is not in its store")));
    let nodes = bed.source::<Node>(Kind::Node, StoreState::Stopped, &[]);
    let stopped = Witness::new([bed.live(&fixture::all::<Pod>()), nodes]);
    assert_eq!(judged(&stopped, &lost), state(Unknown, "the store of the Nodes is not live: stopped"));

    let shortage = warning::<Node>(NODE, "EvictionThresholdMet", "2026-10-13T17:59:00Z", None);
    let pressure = |status: &str, since: &str| {
        with_node(&bed, |node| {
            let disk = condition(node, "DiskPressure");
            (disk["status"], disk["lastTransitionTime"]) = (status.into(), since.into());
        })
    };
    assert_eq!(judged(&pressure("True", EARLY), &shortage), state(Ongoing, "the node has DiskPressure"));
    assert_eq!(judged(&pressure("False", LATE), &shortage), state(Resolved(at(LATE)), "the node has no pressure"));
    let before = state(Unknown, "the node has no pressure since before the last report");
    assert_eq!(judged(&pressure("False", EARLY), &shortage), before);
}

#[test]
fn a_workload_that_could_not_make_its_pods_is_ongoing_until_its_health_is_good() {
    let bed = Bed::new();
    let witness = bed.scene(&fixture::all(), &fixture::all());
    let failed = |name: &str| warning::<ReplicaSet>(name, "FailedCreate", "2026-10-13T17:58:20Z", None);
    let broken = state(Ongoing, "the ReplicaSet is not healthy: warning");
    assert_eq!(judged(&witness, &failed("broken-6bf768c954")), broken, "its one pod is not ready");
    let healthy = "the ReplicaSet is healthy, and its status gives no time of the change";
    assert_eq!(judged(&witness, &failed("web-6888b778b6")), state(Resolved(None), healthy));
}

#[test]
fn an_object_that_went_away_or_came_again_is_gone_and_a_store_that_is_not_live_tells_nothing() {
    let bed = Bed::new();
    let event = warning::<Pod>("crash", "BackOff", "2026-10-13T17:58:53Z", Some("spec.containers{crash}"));
    let again = with_pod(&bed, "crash", |pod| pod["metadata"]["uid"] = "a-new-uid".into());
    assert_eq!(judged(&again, &event), state(Gone, "an other Pod has its name now"));
    let others: Vec<Pod> =
        fixture::all::<Pod>().into_iter().filter(|pod| pod.metadata.name.as_deref() != Some("crash")).collect();
    assert_eq!(judged(&bed.scene(&others, &[]), &event), state(Gone, "the Pod is not in its store"));

    // The old store has no pod crash, and that tells nothing.
    let reason = crate::CoreError::invalid("no answer");
    let paused = StoreState::Backoff { failures: 2, reason, until: SystemTime::UNIX_EPOCH };
    let old = Witness::new([bed.source(Kind::Pod, paused.clone(), &others)]);
    assert_eq!(judged(&old, &event), (Unknown, format!("the store of the Pods is not live: {paused}")));
    let none = Witness::default();
    assert_eq!(judged(&none, &event), state(Unknown, "the cluster has no store of the Pods"));
}

#[test]
fn the_witness_keeps_the_first_full_store_of_a_kind_and_knows_the_same_snapshots() {
    let bed = Bed::new();
    let pods = bed.live(&fixture::all::<Pod>());
    let metas: Vec<Meta> = fixture::all::<Node>().iter().map(|node| Meta { metadata: node.metadata.clone() }).collect();
    let witness = Witness::new([pods.clone(), bed.live::<Pod>(&[]), bed.source(Kind::Node, StoreState::Live, &metas)]);
    assert_eq!(witness.sources.iter().map(|source| source.status.kind).collect::<Vec<_>>(), [Kind::Pod]);
    assert!(witness.store::<Pod>().is_ok_and(|pods| pods.items.len() == 25), "the first store of the Pods");
    assert!(witness.is_same(&Witness::new([pods.clone()])), "one snapshot");
    assert!(!witness.is_same(&Witness::new([bed.live(&fixture::all::<Pod>())])), "a new snapshot");
    assert!(!witness.is_same(&Witness::new([pods, bed.live(&fixture::all::<Job>())])), "one more store");
}
