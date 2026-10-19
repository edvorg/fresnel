//! The rules of a controller that could not make a pod (`FailedCreate`), and of a Job that
//! failed.
//!
//! The controller tried to make a pod at the last report, so it had fewer pods than it
//! wanted then. When it has each one now, it made them after the report: the problem ended,
//! at a time that its status does not give. The readiness of the pods is an other problem,
//! with Events of its own, so it does not count here.

use k8s_openapi::api::apps::v1::{DaemonSet, ReplicaSet, StatefulSet};
use k8s_openapi::api::batch::v1::Job;

use super::ProblemNow;
use crate::kinds::health::{Condition, condition, is_true};

/// The pods of a ReplicaSet. Its controller sets the condition `ReplicaFailure` while it
/// cannot make one.
pub(super) fn replica_set(set: &ReplicaSet) -> ProblemNow {
    let status = set.status.as_ref();
    let conditions = status.and_then(|status| status.conditions.as_ref());
    if let Some(failure) = condition(conditions, "ReplicaFailure").filter(|failure| failure.status() == "True") {
        return ProblemNow::ongoing(format!("the ReplicaSet has the condition ReplicaFailure: {}", failure.reason()));
    }
    // The API gives a spec with no count 1 replica.
    let wanted = set.spec.as_ref().and_then(|spec| spec.replicas).unwrap_or(1);
    let current = current(set.metadata.generation, status.and_then(|status| status.observed_generation));
    made("ReplicaSet", current, status.map(|status| (status.replicas, wanted)), "pods")
}

pub(super) fn stateful_set(set: &StatefulSet) -> ProblemNow {
    let status = set.status.as_ref();
    let wanted = set.spec.as_ref().and_then(|spec| spec.replicas).unwrap_or(1);
    let current = current(set.metadata.generation, status.and_then(|status| status.observed_generation));
    made("StatefulSet", current, status.map(|status| (status.replicas, wanted)), "pods")
}

/// The nodes that run a pod of a DaemonSet, against the nodes that should.
pub(super) fn daemon_set(set: &DaemonSet) -> ProblemNow {
    let status = set.status.as_ref();
    let current = current(set.metadata.generation, status.and_then(|status| status.observed_generation));
    let counts = status.map(|status| (status.current_number_scheduled, status.desired_number_scheduled));
    made("DaemonSet", current, counts, "nodes with its pod")
}

/// The pods of a Job that runs, against the ones that it wants now: its parallelism, to the
/// completions that are left. A Job also waits after a failed pod with fewer pods, so a
/// short count tells nothing.
pub(super) fn job_pods(job: &Job) -> ProblemNow {
    let status = job.status.as_ref();
    let conditions = status.and_then(|status| status.conditions.as_ref());
    if is_true(conditions, "Complete") {
        return ProblemNow::resolved(None, "the Job is complete");
    }
    if is_true(conditions, "Failed") || is_true(conditions, "FailureTarget") {
        return ProblemNow::unknown("the Job failed, so it makes no pod");
    }
    let spec = job.spec.as_ref();
    if spec.and_then(|spec| spec.suspend) == Some(true) || is_true(conditions, "Suspended") {
        return ProblemNow::unknown("the Job is suspended, so it makes no pod now");
    }
    // The API writes no count of 0, and gives a spec with no parallelism 1.
    let (active, succeeded) =
        status.map_or((0, 0), |status| (status.active.unwrap_or(0), status.succeeded.unwrap_or(0)));
    let parallel = spec.and_then(|spec| spec.parallelism).unwrap_or(1);
    let left = spec.and_then(|spec| spec.completions).map(|completions| completions - succeeded);
    let wanted = left.map_or(parallel, |left| left.min(parallel));
    let runs = format!("the Job runs {active} of {wanted} pods");
    if wanted > 0 && active >= wanted {
        ProblemNow::resolved(None, runs)
    } else {
        ProblemNow::unknown(format!("{runs}: a wait after a failed pod looks the same"))
    }
}

/// A Job that failed for good: its condition stays.
pub(super) fn job_failed(job: &Job) -> ProblemNow {
    let conditions = job.status.as_ref().and_then(|status| status.conditions.as_ref());
    let failed = ["Failed", "FailureTarget"]
        .into_iter()
        .find_map(|name| condition(conditions, name).filter(|failed| failed.status() == "True"));
    match failed {
        Some(failed) => ProblemNow::ongoing(format!("the Job failed: {}", failed.reason())),
        None => ProblemNow::unknown("the Job has no condition Failed"),
    }
}

/// True when the controller wrote the status for the newest spec: an older status counts
/// the pods of an other spec.
fn current(generation: Option<i64>, observed: Option<i64>) -> bool {
    generation.zip(observed).is_some_and(|(generation, observed)| observed >= generation)
}

/// The state of the count of what a controller made against what it wants, in `unit`.
fn made(kind: &str, current: bool, counts: Option<(i32, i32)>, unit: &str) -> ProblemNow {
    match counts {
        _ if !current => ProblemNow::unknown(format!("the status of the {kind} is not of its newest spec")),
        None => ProblemNow::unknown(format!("the {kind} has no status")),
        Some((made, wanted)) if made < wanted => {
            ProblemNow::ongoing(format!("the {kind} has {made} of {wanted} {unit}"))
        }
        Some((made, wanted)) => ProblemNow::resolved(None, format!("the {kind} has {made} of {wanted} {unit}")),
    }
}
