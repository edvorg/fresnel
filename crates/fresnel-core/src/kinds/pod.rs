//! The Pod: its description, what its table shows of its state, and its health value.
//!
//! - `status`: the rule of kubectl for the status text.

mod status;

use k8s_openapi::api::core::v1::Pod;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;

use super::health::{ObjectHealth, condition, is_true};
use super::{Action, Column, Describe, Kind, Sidebar, Tier, Value, read};

impl Describe for Pod {
    const ID: Kind = Kind::Pod;
    const TITLE: &'static str = "Pods";
    const SIDEBAR: Sidebar = Sidebar::Workloads;
    const TIER: Tier = Tier::A;
    const COLUMNS: &'static [Column<Self>] = &[
        Column::number("Ready", 56.0, |pod| {
            let status = PodStatus::of(pod);
            Value::Ratio(status.ready.into(), status.containers.into())
        }),
        Column::text("Status", 140.0, |pod| Value::owned(PodStatus::of(pod).text)),
        Column::number("Restarts", 70.0, |pod| Value::Number(PodStatus::of(pod).restarts.into())),
        Column::number("Last restart", 100.0, |pod| match PodStatus::of(pod).last_restart {
            Some(time) => Value::time(Some(&time)),
            None => Value::None,
        }),
        Column::text("IP", 110.0, |pod| Value::text(pod.status.as_ref().and_then(|status| status.pod_ip.as_deref()))),
        Column::text("Node", 130.0, |pod| Value::text(pod.spec.as_ref().and_then(|spec| spec.node_name.as_deref()))),
        Column::text("Controlled by", 160.0, |pod| read::controller(&pod.metadata)),
        Column::text("Containers", 140.0, |pod: &Self| read::containers(pod.spec.as_ref())).hidden(),
        Column::text("QoS", 90.0, |pod: &Self| {
            Value::text(pod.status.as_ref().and_then(|status| status.qos_class.as_deref()))
        })
        .hidden(),
        Column::text("Nominated node", 130.0, |pod: &Self| {
            Value::text(pod.status.as_ref().and_then(|status| status.nominated_node_name.as_deref()))
        })
        .hidden(),
        Column::number("Readiness gates", 80.0, readiness_gates).hidden(),
    ];
    const ACTIONS: &'static [Action] =
        &[Action::Logs, Action::Terminal, Action::Attach, Action::DebugContainer, Action::Forward, Action::Evict];

    fn health(&self) -> ObjectHealth {
        PodStatus::of(self).health
    }

    /// The images, the node and the addresses of the pod and of its node.
    fn search<'a>(&'a self, texts: &mut Vec<&'a str>) {
        let (spec, status) = (self.spec.as_ref(), self.status.as_ref());
        read::search_images(spec, texts);
        texts.extend(spec.and_then(|spec| spec.node_name.as_deref()));
        texts.extend(status.into_iter().flat_map(|status| status.pod_ips.iter().flatten()).map(|ip| ip.ip.as_str()));
        texts.extend(status.and_then(|status| status.host_ip.as_deref()));
    }
}

/// The readiness gates that are true, of all gates: the column `READINESS GATES` of kubectl.
fn readiness_gates(pod: &Pod) -> Value<'static> {
    let gates = pod.spec.as_ref().and_then(|spec| spec.readiness_gates.as_ref()).filter(|gates| !gates.is_empty());
    let Some(gates) = gates else {
        return Value::None;
    };
    let conditions = pod.status.as_ref().and_then(|status| status.conditions.as_ref());
    let ready = gates.iter().filter(|gate| is_true(conditions, &gate.condition_type)).count();
    Value::Ratio(read::number_of(ready), read::number_of(gates.len()))
}

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
