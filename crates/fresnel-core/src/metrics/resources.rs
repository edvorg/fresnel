//! The requests and limits of a pod, and the amount type of the metrics.

use std::collections::BTreeMap;
use std::iter::Sum;
use std::ops::Add;

use k8s_openapi::api::core::v1::{Container, Pod, PodSpec, ResourceRequirements};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;

use super::quantity;

/// An amount of CPU and memory: CPU in nanocores, memory in bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Amount {
    /// CPU in nanocores: one core is 1,000,000,000.
    pub cpu: u64,
    /// Memory in bytes.
    pub memory: u64,
}

impl Add for Amount {
    type Output = Self;

    /// A sum stops at the largest value, so no amount of a cluster can wrap around.
    fn add(self, other: Self) -> Self {
        Self { cpu: self.cpu.saturating_add(other.cpu), memory: self.memory.saturating_add(other.memory) }
    }
}

impl Sum for Amount {
    fn sum<I: Iterator<Item = Self>>(amounts: I) -> Self {
        amounts.fold(Self::default(), Add::add)
    }
}

/// The limits of a pod. `None` for a resource with no limit: a container with no limit
/// can use the whole node, and so can its pod (the rule of the kubelet for the cgroup of a
/// pod).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Limits {
    /// The CPU limit in nanocores.
    pub cpu: Option<u64>,
    /// The memory limit in bytes.
    pub memory: Option<u64>,
}

/// The requests and the limits of a pod.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PodResources {
    /// What the scheduler reserves on the node.
    pub requests: Amount,
    /// The most that the pod can use.
    pub limits: Limits,
}

/// The requests and the limits of a pod, with the rule of the API (`PodRequests` and
/// `PodLimits` of `k8s.io/component-helpers`): the sum of the containers and the sidecars,
/// or the largest step of the init containers when it is larger. A value of the pod level
/// wins, and the overhead of the runtime class adds to it.
pub fn of_pod(pod: &Pod) -> PodResources {
    let Some(spec) = pod.spec.as_ref() else {
        return PodResources::default();
    };
    let requests = |name, read| value(spec, Side::Requests, name, read).unwrap_or_default();
    let limit = |name, read| value(spec, Side::Limits, name, read);
    PodResources {
        requests: Amount { cpu: requests("cpu", quantity::nanocores), memory: requests("memory", quantity::bytes) },
        limits: Limits { cpu: limit("cpu", quantity::nanocores), memory: limit("memory", quantity::bytes) },
    }
}

/// True for a pod in the phase `Succeeded` or `Failed`. Such a pod holds no resource of its
/// node, so the requests of a node do not count it (the rule of `kubectl describe node`),
/// and the requests of a workload in a view do not either.
pub fn is_done(pod: &Pod) -> bool {
    let phase = pod.status.as_ref().and_then(|status| status.phase.as_deref());
    matches!(phase, Some("Succeeded" | "Failed"))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Requests,
    Limits,
}

impl Side {
    fn of(self, resources: &ResourceRequirements) -> Option<&BTreeMap<String, Quantity>> {
        match self {
            Self::Requests => resources.requests.as_ref(),
            Self::Limits => resources.limits.as_ref(),
        }
    }
}

type Reader = fn(&str) -> Result<u64, &'static str>;

/// The value of one resource of a pod. The server checks each quantity, so a value that the
/// reader cannot read does not come from it; such a value counts as not set.
fn value(spec: &PodSpec, side: Side, name: &str, read: Reader) -> Option<u64> {
    let of = |resources: Option<&ResourceRequirements>| {
        let quantity = resources.and_then(|resources| side.of(resources)).and_then(|values| values.get(name));
        quantity.and_then(|quantity| read(&quantity.0).ok())
    };
    let overhead = spec.overhead.as_ref().and_then(|values| values.get(name)).and_then(|q| read(&q.0).ok());
    let with_overhead = |value: u64| Some(value.saturating_add(overhead.unwrap_or_default()));
    if let Some(pod_level) = of(spec.resources.as_ref()) {
        return with_overhead(pod_level);
    }
    let container = |container: &Container| of(container.resources.as_ref());
    let inits = spec.init_containers.as_deref().unwrap_or_default();
    if side == Side::Limits && spec.containers.iter().chain(inits).any(|one| container(one).is_none()) {
        return None;
    }
    let mut sum = spec.containers.iter().filter_map(container).fold(0, u64::saturating_add);
    let (mut sidecars, mut init_peak) = (0u64, 0u64);
    for init in inits {
        let own = container(init).unwrap_or_default();
        let step = if init.restart_policy.as_deref() == Some("Always") {
            sum = sum.saturating_add(own);
            sidecars = sidecars.saturating_add(own);
            sidecars
        } else {
            own.saturating_add(sidecars)
        };
        init_peak = init_peak.max(step);
    }
    with_overhead(sum.max(init_peak))
}

#[cfg(test)]
mod tests;
