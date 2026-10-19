//! The rules of the state of a problem: for the kind of the object and the reason of a
//! warning Event, the fact of the live object that tells if the problem is still there.
//!
//! Each line of `TABLE` gives its reason. A reason with no line stays "unknown": the app
//! does not judge what it cannot read. A healthy state proves an end only when it began
//! after the last report; a state from before it was there at that report too.

use k8s_openapi::api::core::v1::{ContainerStatus, Node, Pod};
use k8s_openapi::jiff::Timestamp;

use super::{ProblemNow, ProblemState, Subject, Witness};
use crate::kinds::health::{Condition, condition, is_true};
use crate::kinds::{Describe, Kind, ObjectHealth, Visit};
use crate::store::ObjectKey;

/// What the rule of a line reads.
#[derive(Clone, Copy)]
enum Rule {
    /// A condition of the pod that is true when the problem is gone.
    Pod(Wanted),
    /// The container that the field path of the Event names.
    Container,
    /// The condition `Ready` of the node of the pod.
    NodeOfPod,
    /// The conditions of a shortage of the node.
    Pressure,
    /// The health value of the workload.
    Health,
}

/// A condition that is true when the problem is gone, with the words of its two states.
#[derive(Clone, Copy)]
struct Wanted {
    name: &'static str,
    bad: &'static str,
    good: &'static str,
}

const READY: Wanted = Wanted { name: "Ready", bad: "is not ready", good: "is ready" };
const SCHEDULED: Wanted = Wanted { name: "PodScheduled", bad: "waits for a node", good: "has a node" };
const SANDBOX: Wanted = Wanted { name: "PodReadyToStartContainers", bad: "has no sandbox", good: "has its sandbox" };
/// The conditions of a node that are true during a shortage.
const PRESSURES: [&str; 3] = ["MemoryPressure", "DiskPressure", "PIDPressure"];

/// One line of the table: the kind of the object, the reasons of its Events and the rule.
struct Line {
    kind: Kind,
    reasons: &'static [&'static str],
    rule: Rule,
}

const TABLE: [Line; 10] = [
    // The kubelet reports a failed probe. A failed readiness probe, and the restart after a
    // failed liveness or startup probe, make the pod not ready.
    Line { kind: Kind::Pod, reasons: &["Unhealthy"], rule: Rule::Pod(READY) },
    // The kubelet cannot start a container, or it crashed: the field path names it.
    Line { kind: Kind::Pod, reasons: &["BackOff", "Failed"], rule: Rule::Container },
    // The scheduler finds no node; the condition stays false until it finds one.
    Line { kind: Kind::Pod, reasons: &["FailedScheduling"], rule: Rule::Pod(SCHEDULED) },
    // The kubelet cannot make the sandbox and the network of the pod. Kubernetes 1.29 and
    // newer give the condition.
    Line { kind: Kind::Pod, reasons: &["FailedCreatePodSandBox"], rule: Rule::Pod(SANDBOX) },
    // The node controller marks each pod of a node that stopped its reports.
    Line { kind: Kind::Pod, reasons: &["NodeNotReady"], rule: Rule::NodeOfPod },
    // The kubelet evicts pods while its node reports a shortage of memory, disk or PIDs.
    Line { kind: Kind::Node, reasons: &["EvictionThresholdMet"], rule: Rule::Pressure },
    // A controller cannot make a pod, or a Job failed: the workload is not healthy until it
    // has its pods, and a healthy one has no such problem left.
    Line { kind: Kind::ReplicaSet, reasons: &["FailedCreate"], rule: Rule::Health },
    Line { kind: Kind::StatefulSet, reasons: &["FailedCreate"], rule: Rule::Health },
    Line { kind: Kind::DaemonSet, reasons: &["FailedCreate"], rule: Rule::Health },
    Line {
        kind: Kind::Job,
        reasons: &["FailedCreate", "BackoffLimitExceeded", "DeadlineExceeded"],
        rule: Rule::Health,
    },
];

/// The state of the problem of an Event whose object has a store in the witness.
pub(super) fn judge(witness: &Witness, reason: &str, subject: &Subject<'_>) -> ProblemNow {
    let line = TABLE.iter().find(|line| line.kind == subject.kind && line.reasons.contains(&reason));
    let pod = || witness.find::<Pod>(&subject.key, subject.uid);
    let last = subject.last;
    let judged = match line.map(|line| line.rule) {
        Some(Rule::Pod(wanted)) => {
            pod().map(|pod| of_pod(pod, |pod| by_condition(conditions(pod), wanted, last, "the pod")))
        }
        Some(Rule::Container) => pod().map(|pod| of_pod(pod, |pod| container(pod, subject))),
        Some(Rule::NodeOfPod) => pod().map(|pod| of_pod(pod, |pod| node_of(witness, pod, last))),
        Some(Rule::Pressure) => witness.find::<Node>(&subject.key, subject.uid).map(|node| pressure(node, last)),
        Some(Rule::Health) => subject.kind.visit(Health(witness, subject)),
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
fn of_pod(pod: &Pod, rule: impl FnOnce(&Pod) -> ProblemNow) -> ProblemNow {
    match pod.status.as_ref().and_then(|status| status.phase.as_deref()) {
        Some("Succeeded") => ProblemNow::resolved(None, "the pod ended well"),
        Some("Failed") => ProblemNow::unknown("the pod failed, so no container of it runs"),
        _ => rule(pod),
    }
}

fn conditions(pod: &Pod) -> Option<&Vec<k8s_openapi::api::core::v1::PodCondition>> {
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
/// before it was there at the report too, so the problem did not end it.
fn after(since: Option<Timestamp>, last: Option<Timestamp>, healthy: String) -> ProblemNow {
    match (since, last) {
        (Some(since), Some(last)) if since > last => ProblemNow::resolved(Some(since), healthy),
        (Some(_), Some(_)) => ProblemNow::unknown(format!("{healthy} since before the last report")),
        (None, _) => ProblemNow::unknown(format!("{healthy}, and its status gives no time of the change")),
        (Some(_), None) => ProblemNow::unknown(format!("{healthy}, and the Event gives no time of its report")),
    }
}

/// The container of the field path, as `spec.containers{web}`. A pod has one name for
/// each container, init container and ephemeral container.
fn container(pod: &Pod, subject: &Subject<'_>) -> ProblemNow {
    let name = subject.part.and_then(|part| part.split_once('{')).and_then(|(_, rest)| rest.strip_suffix('}'));
    let Some(name) = name else {
        return ProblemNow::unknown("the Event names no container");
    };
    let status = pod.status.as_ref();
    let lists = status.into_iter().flat_map(|status| {
        [&status.container_statuses, &status.init_container_statuses, &status.ephemeral_container_statuses]
    });
    let found = lists.flatten().flatten().find(|container: &&ContainerStatus| container.name == name);
    let state = found.and_then(|container| container.state.as_ref());
    let the = format!("the container {name}");
    match state.map(|state| (&state.waiting, &state.running, &state.terminated)) {
        Some((Some(waiting), _, _)) => match &waiting.reason {
            Some(reason) => ProblemNow::ongoing(format!("{the} waits: {reason}")),
            None => ProblemNow::ongoing(format!("{the} waits")),
        },
        Some((None, Some(running), _)) => {
            after(running.started_at.as_ref().map(|time| time.0), subject.last, format!("{the} runs"))
        }
        Some((None, None, Some(ended))) if ended.exit_code == 0 => {
            let started = ended.started_at.as_ref().or(ended.finished_at.as_ref()).map(|time| time.0);
            after(started, subject.last, format!("{the} ended well"))
        }
        Some((None, None, Some(ended))) => {
            ProblemNow::ongoing(format!("{the} ended with exit code {}", ended.exit_code))
        }
        Some((None, None, None)) | None => ProblemNow::unknown(format!("the pod gives no state of {the}")),
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
        Err(ProblemNow { state: ProblemState::Gone, .. }) => {
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

/// The health value of a workload. A warning of a controller tells that the workload was
/// not complete at its report, so a healthy one became so after it, at a time that its
/// status does not give.
struct Health<'a>(&'a Witness, &'a Subject<'a>);

impl Visit for Health<'_> {
    type Output = Result<ProblemNow, ProblemNow>;

    fn visit<K: Describe>(self) -> Self::Output {
        let object = self.0.find::<K>(&self.1.key, self.1.uid)?;
        let word = match object.health() {
            ObjectHealth::Healthy => {
                let why = format!("the {} is healthy, and its status gives no time of the change", K::KIND);
                return Ok(ProblemNow::resolved(None, why));
            }
            ObjectHealth::Progressing => "progressing",
            ObjectHealth::Warning => "warning",
            ObjectHealth::Failed => "failed",
        };
        Ok(ProblemNow::ongoing(format!("the {} is not healthy: {word}", K::KIND)))
    }
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
