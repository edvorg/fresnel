//! The rules of the state of a problem: for the kind of the object and the reason of a
//! warning Event, the fact of the live object that tells if the problem is still there.
//!
//! Each line of `TABLE` gives its reason. A reason with no line stays "unknown": the app
//! does not judge what it cannot read. A healthy state proves an end only when it began
//! after the last report; a state from before it was there at that report too.

use k8s_openapi::api::apps::v1::{DaemonSet, ReplicaSet, StatefulSet};
use k8s_openapi::api::batch::v1::Job;
use k8s_openapi::api::core::v1::{Node, Pod, PodCondition};
use k8s_openapi::jiff::Timestamp;

use super::{ProblemNow, ProblemState, Subject, Witness, container, workload};
use crate::kinds::health::{Condition, condition, is_true};
use crate::kinds::{Describe, Kind, Visit};
use crate::store::ObjectKey;

/// What the rule of a line reads.
#[derive(Clone, Copy)]
enum Rule {
    /// A condition of the pod that is true when the problem is gone.
    Pod(Wanted),
    /// The container that the field path of the Event names.
    Container,
    /// The probe that the message names, in the container of the field path.
    Probe,
    /// The condition `Ready` of the node of the pod.
    NodeOfPod,
    /// The conditions of a shortage of the node.
    Pressure,
    /// The pods that the controller has made, against the ones that it wants.
    ReplicaSetPods,
    StatefulSetPods,
    DaemonSetPods,
    JobPods,
    /// The condition `Failed` of a Job.
    JobFailed,
}

/// A condition that is true when the problem is gone, with the words of its two states.
#[derive(Clone, Copy)]
pub(super) struct Wanted {
    pub(super) name: &'static str,
    bad: &'static str,
    good: &'static str,
}

pub(super) const READY: Wanted = Wanted { name: "Ready", bad: "is not ready", good: "is ready" };
const SCHEDULED: Wanted = Wanted { name: "PodScheduled", bad: "waits for a node", good: "has a node" };
const SANDBOX: Wanted = Wanted { name: "PodReadyToStartContainers", bad: "has no sandbox", good: "has its sandbox" };
const VOLUMES: Wanted = Wanted {
    name: "PodReadyToStartContainers",
    bad: "waits for its volumes and its sandbox",
    good: "has its volumes and its sandbox",
};
/// The conditions of a node that are true during a shortage.
const PRESSURES: [&str; 3] = ["MemoryPressure", "DiskPressure", "PIDPressure"];

/// One line of the table: the kind of the object, the reasons of its Events and the rule.
struct Line {
    kind: Kind,
    reasons: &'static [&'static str],
    rule: Rule,
}

const TABLE: [Line; 12] = [
    // The kubelet reports a failed probe, and the message names it: a readiness probe makes
    // its container not ready, a liveness or startup probe restarts it.
    Line { kind: Kind::Pod, reasons: &["Unhealthy"], rule: Rule::Probe },
    // The kubelet cannot start a container, or it crashed: the field path names it.
    Line { kind: Kind::Pod, reasons: &["BackOff", "Failed"], rule: Rule::Container },
    // The scheduler finds no node; the condition stays false until it finds one.
    Line { kind: Kind::Pod, reasons: &["FailedScheduling"], rule: Rule::Pod(SCHEDULED) },
    // The kubelet cannot make the sandbox and the network of the pod. Kubernetes 1.29 and
    // newer give the condition.
    Line { kind: Kind::Pod, reasons: &["FailedCreatePodSandBox", "NetworkNotReady"], rule: Rule::Pod(SANDBOX) },
    // The volumes of the pod: the kubelet makes the sandbox only after it mounted each one.
    Line { kind: Kind::Pod, reasons: &["FailedMount", "FailedAttachVolume"], rule: Rule::Pod(VOLUMES) },
    // The node controller marks each pod of a node that stopped its reports.
    Line { kind: Kind::Pod, reasons: &["NodeNotReady"], rule: Rule::NodeOfPod },
    // The kubelet evicts pods while its node reports a shortage of memory, disk or PIDs.
    Line { kind: Kind::Node, reasons: &["EvictionThresholdMet"], rule: Rule::Pressure },
    // A controller cannot make a pod: it has fewer pods than it wants until it can. Their
    // readiness is an other problem.
    Line { kind: Kind::ReplicaSet, reasons: &["FailedCreate"], rule: Rule::ReplicaSetPods },
    Line { kind: Kind::StatefulSet, reasons: &["FailedCreate"], rule: Rule::StatefulSetPods },
    Line { kind: Kind::DaemonSet, reasons: &["FailedCreate"], rule: Rule::DaemonSetPods },
    Line { kind: Kind::Job, reasons: &["FailedCreate"], rule: Rule::JobPods },
    // A Job failed for good: its condition stays.
    Line { kind: Kind::Job, reasons: &["BackoffLimitExceeded", "DeadlineExceeded"], rule: Rule::JobFailed },
];

/// The state of the problem of an Event whose object has a store in the witness.
pub(super) fn judge(witness: &Witness, reason: &str, subject: &Subject<'_>) -> ProblemNow {
    let line = TABLE.iter().find(|line| line.kind == subject.kind && line.reasons.contains(&reason));
    let (key, uid, last) = (&subject.key, subject.uid, subject.last);
    let pod = || witness.find::<Pod>(key, uid);
    let judged = match line.map(|line| line.rule) {
        Some(Rule::Pod(wanted)) => {
            pod().map(|pod| of_pod(pod, |pod| by_condition(conditions(pod), wanted, last, "the pod")))
        }
        Some(Rule::Container) => pod().map(|pod| of_pod(pod, |pod| container::container(pod, subject, witness.at))),
        Some(Rule::Probe) => pod().map(|pod| of_pod(pod, |pod| container::probe(pod, subject, witness.at))),
        Some(Rule::NodeOfPod) => pod().map(|pod| of_pod(pod, |pod| node_of(witness, pod, last))),
        Some(Rule::Pressure) => witness.find::<Node>(key, uid).map(|node| pressure(node, last)),
        Some(Rule::ReplicaSetPods) => witness.find::<ReplicaSet>(key, uid).map(workload::replica_set),
        Some(Rule::StatefulSetPods) => witness.find::<StatefulSet>(key, uid).map(workload::stateful_set),
        Some(Rule::DaemonSetPods) => witness.find::<DaemonSet>(key, uid).map(workload::daemon_set),
        Some(Rule::JobPods) => witness.find::<Job>(key, uid).map(workload::job_pods),
        Some(Rule::JobFailed) => witness.find::<Job>(key, uid).map(workload::job_failed),
        None => subject.kind.visit(Present(witness, subject)).map(|()| no_rule(reason, subject.kind.info().name)),
    };
    match judged {
        Ok(now) | Err(now) => now,
    }
}

/// The state of an Event that no line of the table judges.
pub(super) fn no_rule(reason: &str, kind: &str) -> ProblemNow {
    ProblemNow::unknown(format!("the app has no rule for the reason {reason} of a {kind}"))
}

/// The rule of a pod that runs or waits. A pod that ended runs no container: a pod that
/// ended well left each problem behind, and the state of a failed one tells nothing of one.
/// A pod that starts its containers again ends only when it is stopped, as by a delete.
fn of_pod(pod: &Pod, rule: impl FnOnce(&Pod) -> ProblemNow) -> ProblemNow {
    let policy = pod.spec.as_ref().and_then(|spec| spec.restart_policy.as_deref());
    let ends = matches!(policy, Some("Never" | "OnFailure"));
    match pod.status.as_ref().and_then(|status| status.phase.as_deref()) {
        Some("Succeeded") if ends => ProblemNow::resolved(None, "the pod ended well"),
        Some("Succeeded") => ProblemNow::unknown("the pod was stopped, so no container of it runs"),
        Some("Failed") => ProblemNow::unknown("the pod failed, so no container of it runs"),
        _ => rule(pod),
    }
}

pub(super) fn conditions(pod: &Pod) -> Option<&Vec<PodCondition>> {
    pod.status.as_ref()?.conditions.as_ref()
}

/// The rule of a condition that is true when the problem is gone, of `object` ("the pod").
fn by_condition<C: Condition>(
    conditions: Option<&Vec<C>>,
    wanted: Wanted,
    last: Option<Timestamp>,
    object: &str,
) -> ProblemNow {
    match condition(conditions, wanted.name) {
        Some(found) if found.status() == "True" => after(found.since(), last, format!("{object} {}", wanted.good)),
        Some(_) => ProblemNow::ongoing(format!("{object} {}", wanted.bad)),
        None => ProblemNow::unknown(format!("{object} has no condition {}", wanted.name)),
    }
}

/// "Resolved" when the healthy state `healthy` began after the last report. One that began
/// before it was there at the report too, so the problem did not end it: the object then
/// proves neither an end nor that the problem goes on.
pub(super) fn after(since: Option<Timestamp>, last: Option<Timestamp>, healthy: String) -> ProblemNow {
    match (since, last) {
        (Some(since), Some(last)) if since > last => ProblemNow::resolved(Some(since), healthy),
        (Some(_), Some(_)) => ProblemNow::undecided(format!("{healthy} since before the last report")),
        (None, _) => ProblemNow::undecided(format!("{healthy}, and its status gives no time of the change")),
        (Some(_), None) => ProblemNow::undecided(format!("{healthy}, and the Event gives no time of its report")),
    }
}

/// The node of a pod and its condition `Ready`. The pod is no witness of its node.
fn node_of(witness: &Witness, pod: &Pod, last: Option<Timestamp>) -> ProblemNow {
    let Some(name) = pod.spec.as_ref().and_then(|spec| spec.node_name.as_deref()) else {
        return ProblemNow::unknown("the pod has no node");
    };
    match witness.find::<Node>(&ObjectKey::new("", name), None) {
        Ok(node) => {
            let conditions = node.status.as_ref().and_then(|status| status.conditions.as_ref());
            by_condition(conditions, READY, last, &format!("the node {name}"))
        }
        Err(ProblemNow { state: ProblemState::Deleted, .. }) => {
            ProblemNow::unknown(format!("the node {name} is not in its store"))
        }
        Err(other) => other,
    }
}

/// A shortage of the node: each condition of a pressure is false after the last report.
fn pressure(node: &Node, last: Option<Timestamp>) -> ProblemNow {
    let conditions = node.status.as_ref().and_then(|status| status.conditions.as_ref());
    if let Some(found) = PRESSURES.iter().find(|name| is_true(conditions, name)) {
        return ProblemNow::ongoing(format!("the node has {found}"));
    }
    let known: Vec<_> = PRESSURES.iter().filter_map(|name| condition(conditions, name)).collect();
    if known.is_empty() {
        return ProblemNow::unknown("the node has no condition of a pressure");
    }
    // The last change of them, when each one has its time.
    let since = known.iter().map(|condition| condition.since()).collect::<Option<Vec<_>>>();
    after(since.and_then(|times| times.into_iter().max()), last, "the node has no pressure".to_owned())
}

/// The object of the Event in its live store, for an Event with no rule: a store that is
/// not live, and an object that went away, tell their own state.
struct Present<'a>(&'a Witness, &'a Subject<'a>);

impl Visit for Present<'_> {
    type Output = Result<(), ProblemNow>;

    fn visit<K: Describe>(self) -> Self::Output {
        self.0.find::<K>(&self.1.key, self.1.uid).map(|_| ())
    }
}
