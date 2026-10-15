//! What an action needs to know of its object besides its identity: the count of pods, a
//! suspend, a pause, a Job that runs, the strategy, a cordon, and the containers and ports
//! for the `kubectl` commands. The view reads them from the object of its store when the user asks.

use std::any::Any;

use fresnel_core::kinds::Describe;
use k8s_openapi::api::apps::v1::{DaemonSet, Deployment, ReplicaSet, StatefulSet};
use k8s_openapi::api::batch::v1::{CronJob, Job};
use k8s_openapi::api::core::v1::{Container, Node, Pod, PodSpec, ReplicationController, Service};

/// The facts of one object.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Facts {
    /// The count of pods of the spec of a workload with a scale.
    pub(crate) replicas: Option<i32>,
    /// The suspend of a Job or a CronJob. `None` for an other kind.
    pub(crate) suspended: Option<bool>,
    /// A paused Deployment: kubectl refuses its restart and its rollback.
    pub(crate) paused: bool,
    /// A Job that did not end: "run again" stops its pods.
    pub(crate) running: bool,
    /// A pod of a StatefulSet: a force delete can give two pods with one identity.
    pub(crate) stateful_pod: bool,
    /// A StatefulSet or a DaemonSet with the update strategy `OnDelete`: its pods keep the
    /// old template after a rollback until someone deletes them.
    pub(crate) on_delete: bool,
    /// The concurrency policy of a CronJob.
    pub(crate) concurrency: Option<String>,
    /// A node that takes no new pod. `None` for an other kind.
    pub(crate) cordoned: Option<bool>,
    /// The containers, for the commands of logs and exec.
    pub(crate) containers: Vec<String>,
    /// The ports of the containers or of a Service, for the commands of a port forward.
    pub(crate) ports: Vec<u16>,
}

/// The facts of an object of any kind. A kind with none of them gets the defaults.
pub(crate) fn of<K: Describe>(object: &K) -> Facts {
    let any = object as &dyn Any;
    if let Some(pod) = any.downcast_ref::<Pod>() {
        let owners = pod.metadata.owner_references.iter().flatten();
        let stateful_pod = owners.into_iter().any(|owner| owner.kind == "StatefulSet");
        return Facts { stateful_pod, ..of_spec(pod.spec.as_ref()) };
    }
    if let Some(set) = any.downcast_ref::<Deployment>() {
        let spec = set.spec.as_ref();
        let template = of_spec(spec.and_then(|spec| spec.template.spec.as_ref()));
        let replicas = Some(spec.and_then(|spec| spec.replicas).unwrap_or(1));
        return Facts { replicas, paused: spec.and_then(|spec| spec.paused) == Some(true), ..template };
    }
    if let Some(set) = any.downcast_ref::<StatefulSet>() {
        let spec = set.spec.as_ref();
        let strategy = spec.and_then(|spec| spec.update_strategy.as_ref()?.type_.as_deref());
        let replicas = Some(spec.and_then(|spec| spec.replicas).unwrap_or(1));
        let template = of_spec(spec.and_then(|spec| spec.template.spec.as_ref()));
        return Facts { replicas, on_delete: strategy == Some("OnDelete"), ..template };
    }
    if let Some(set) = any.downcast_ref::<DaemonSet>() {
        let spec = set.spec.as_ref();
        let strategy = spec.and_then(|spec| spec.update_strategy.as_ref()?.type_.as_deref());
        let template = of_spec(spec.and_then(|spec| spec.template.spec.as_ref()));
        return Facts { on_delete: strategy == Some("OnDelete"), ..template };
    }
    if let Some(set) = any.downcast_ref::<ReplicaSet>() {
        let spec = set.spec.as_ref();
        let template = of_spec(spec.and_then(|spec| spec.template.as_ref()?.spec.as_ref()));
        return Facts { replicas: Some(spec.and_then(|spec| spec.replicas).unwrap_or(1)), ..template };
    }
    if let Some(set) = any.downcast_ref::<ReplicationController>() {
        let spec = set.spec.as_ref();
        let template = of_spec(spec.and_then(|spec| spec.template.as_ref()?.spec.as_ref()));
        return Facts { replicas: Some(spec.and_then(|spec| spec.replicas).unwrap_or(1)), ..template };
    }
    if let Some(job) = any.downcast_ref::<Job>() {
        let spec = job.spec.as_ref();
        let conditions = job.status.as_ref().and_then(|status| status.conditions.as_ref());
        let ended = conditions
            .into_iter()
            .flatten()
            .any(|condition| matches!(condition.type_.as_str(), "Complete" | "Failed") && condition.status == "True");
        let suspended = Some(spec.and_then(|spec| spec.suspend) == Some(true));
        return Facts { suspended, running: !ended, ..of_spec(spec.and_then(|spec| spec.template.spec.as_ref())) };
    }
    if let Some(cron_job) = any.downcast_ref::<CronJob>() {
        let spec = cron_job.spec.as_ref();
        let suspended = Some(spec.and_then(|spec| spec.suspend) == Some(true));
        let concurrency = Some(spec.and_then(|spec| spec.concurrency_policy.clone()).unwrap_or("Allow".to_owned()));
        // kubectl has no logs and no port forward of a CronJob: its Jobs come and go.
        return Facts { suspended, concurrency, ..Facts::default() };
    }
    if let Some(node) = any.downcast_ref::<Node>() {
        let cordoned = node.spec.as_ref().and_then(|spec| spec.unschedulable) == Some(true);
        return Facts { cordoned: Some(cordoned), ..Facts::default() };
    }
    if let Some(service) = any.downcast_ref::<Service>() {
        let ports = service.spec.as_ref().and_then(|spec| spec.ports.as_ref());
        let ports = ports.into_iter().flatten().filter_map(|port| u16::try_from(port.port).ok()).collect();
        return Facts { ports, ..Facts::default() };
    }
    Facts::default()
}

/// The containers and the ports of a pod spec.
fn of_spec(spec: Option<&PodSpec>) -> Facts {
    let containers: &[Container] = spec.map(|spec| spec.containers.as_slice()).unwrap_or_default();
    let ports = containers.iter().flat_map(|container| container.ports.iter().flatten());
    Facts {
        containers: containers.iter().map(|container| container.name.clone()).collect(),
        ports: ports.filter_map(|port| u16::try_from(port.container_port).ok()).collect(),
        ..Facts::default()
    }
}
