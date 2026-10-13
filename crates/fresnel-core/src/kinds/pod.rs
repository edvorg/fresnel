//! The Pod: what its table shows of its state, and its health value.
//!
//! - `status`: the rule of kubectl for the status text.

mod status;

use k8s_openapi::api::core::v1::Pod;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;

use super::health::{Assess, ObjectHealth, condition, is_true};

/// The state of one pod as the Pod table shows it (plan section 11.2): the columns Ready,
/// Status, Restarts and Last restart, with the values of `kubectl get pods`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PodStatus {
    /// The status text, for example `Running`, `CrashLoopBackOff` or `Init:1/2`.
    pub text: String,
    /// The count of containers that are ready.
    pub ready: u32,
    /// The count of containers that can be ready: each container, and each init container
    /// that runs for the life of the pod.
    pub containers: u32,
    /// The sum of the restarts of the containers.
    pub restarts: u32,
    /// The end of the last run of a container before a restart. `None` with no restart.
    pub last_restart: Option<Time>,
    /// The health value of the pod. It follows from the text, so a row computes the two
    /// in one step.
    pub health: ObjectHealth,
}

impl PodStatus {
    /// Reads the state of a pod.
    pub fn of(pod: &Pod) -> Self {
        let printed = status::print(pod);
        let health = health(pod, &printed.text);
        Self {
            text: printed.text,
            ready: printed.ready,
            containers: printed.containers,
            restarts: printed.restarts,
            last_restart: printed.last_restart,
            health,
        }
    }
}

impl Assess for Pod {
    fn health(&self) -> ObjectHealth {
        PodStatus::of(self).health
    }
}

/// The health value of a pod with this status text. The phase decides for a pod that
/// ended. For each other pod, the text names the state:
///
/// - `Running` is healthy when the pod is ready, and a warning when it is not.
/// - A text of a normal step on the way is progress: the pod starts, waits for its init
///   containers or for a scheduling gate, or ends. `Pending` is progress too, but a
///   warning when the scheduler found no node.
/// - Each other text is the reason of a container that waits or ended, for example
///   `CrashLoopBackOff` or `ErrImagePull`: the pod failed.
fn health(pod: &Pod, text: &str) -> ObjectHealth {
    let status = pod.status.as_ref();
    let conditions = status.and_then(|status| status.conditions.as_ref());
    match status.and_then(|status| status.phase.as_deref()) {
        Some("Succeeded") => return ObjectHealth::Healthy,
        Some("Failed") => return ObjectHealth::Failed,
        _ => {}
    }
    match text {
        "Running" if is_true(conditions, "Ready") => ObjectHealth::Healthy,
        "Running" | "NotReady" | "Completed" => ObjectHealth::Warning,
        "Pending" => match condition(conditions, "PodScheduled").map(|scheduled| scheduled.status.as_str()) {
            Some("False") => ObjectHealth::Warning,
            _ => ObjectHealth::Progressing,
        },
        "ContainerCreating" | "PodInitializing" | "SchedulingGated" | "Terminating" => ObjectHealth::Progressing,
        _ if is_init_count(text) => ObjectHealth::Progressing,
        _ => ObjectHealth::Failed,
    }
}

/// True for the text `Init:<done>/<all>`: the init containers run, with no fault.
fn is_init_count(text: &str) -> bool {
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    text.strip_prefix("Init:")
        .and_then(|count| count.split_once('/'))
        .is_some_and(|(done, all)| digits(done) && digits(all))
}

#[cfg(test)]
mod tests;
