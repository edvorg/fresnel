//! The containers of the pods of a view: which ones the view shows, and what the server
//! gives for the log of one of them now (research 08, section 6.1).

use std::collections::BTreeMap;
use std::sync::Arc;

use k8s_openapi::api::core::v1::{ContainerState, ContainerStateTerminated, ContainerStatus, Pod};

use super::request::Containers;

/// The annotation of the default container of kubectl.
const DEFAULT_ANNOTATION: &str = "kubectl.kubernetes.io/default-container";

/// The names of the containers of a pod in the sequence of kubectl: the init containers,
/// the containers and the ephemeral containers.
pub(super) fn names(pod: &Pod) -> Vec<Arc<str>> {
    let Some(spec) = pod.spec.as_ref() else {
        return Vec::new();
    };
    let init = spec.init_containers.iter().flatten().map(|container| &container.name);
    let ephemeral = spec.ephemeral_containers.iter().flatten().map(|container| &container.name);
    init.chain(spec.containers.iter().map(|container| &container.name))
        .chain(ephemeral)
        .map(|name| name.as_str().into())
        .collect()
}

/// The default container of the pods of a view (plan section 12.7): the container that
/// restarts most in all of them. With no restart, the rule of kubectl: the container of the
/// annotation of the first pod that has the annotation, else the first container of the
/// first pod. `None` with no pod.
pub(super) fn default_of(pods: &[&Pod]) -> Option<Arc<str>> {
    let mut restarts: BTreeMap<&str, i64> = BTreeMap::new();
    for status in pods.iter().flat_map(|pod| statuses(pod)) {
        *restarts.entry(&status.name).or_default() += i64::from(status.restart_count);
    }
    let most = restarts
        .into_iter()
        .filter(|(_, count)| *count > 0)
        .max_by_key(|(name, count)| (*count, std::cmp::Reverse(*name)));
    if let Some((name, _)) = most {
        return Some(name.into());
    }
    let named = pods.iter().find_map(|pod| {
        let name = pod.metadata.annotations.as_ref()?.get(DEFAULT_ANNOTATION)?;
        names(pod).into_iter().find(|container| **container == **name)
    });
    let first = || pods.first()?.spec.as_ref()?.containers.first().map(|container| container.name.as_str().into());
    named.or_else(first)
}

/// The containers of one pod that a view shows. A named container that the pod does not
/// have stays: its stream tells that the pod has no such container.
pub(super) fn chosen(pod: &Pod, containers: &Containers, default: Option<&Arc<str>>) -> Vec<Arc<str>> {
    match containers {
        Containers::Default => default.cloned().into_iter().collect(),
        Containers::Named(name) => vec![Arc::clone(name)],
        Containers::All => names(pod),
    }
}

/// What the server gives now for the log of a container.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Served {
    /// The log of the instance with this ID in the container runtime.
    Instance {
        /// The ID. Each start of the container has a new one.
        id: Arc<str>,
        /// True while the instance runs: its log grows.
        running: bool,
    },
    /// No log: the container did not start, or it has no earlier instance. The reason.
    Nothing(String),
}

/// One container of a pod as its stream sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Run {
    /// What the server gives for the log.
    pub(super) served: Served,
    /// True when the pod starts the container again after its end.
    pub(super) starts_again: bool,
    /// The end of the last instance in words, for example `Error, exit code 1`.
    pub(super) end: Option<String>,
}

/// The container `name` of a pod. `None` when the pod has no container of that name.
///
/// The instance is the one of the rule of the kubelet (`validateContainerLogStatus`): the
/// running one, else the one that ended; with `previous`, the one before it.
pub(super) fn run_of(pod: &Pod, name: &str, previous: bool) -> Option<Run> {
    if !names(pod).iter().any(|container| &**container == name) {
        return None;
    }
    let status = statuses(pod).find(|status| status.name == name);
    let state = status.and_then(|status| status.state.as_ref());
    let last = status.and_then(|status| terminated(status.last_state.as_ref()));
    let ended = terminated(state).filter(|run| has_id(run)).or(last);
    let id = |run: &ContainerStateTerminated| run.container_id.as_deref().filter(|id| !id.is_empty()).map(Arc::from);
    let served = if previous {
        last.and_then(id).map(|id| Served::Instance { id, running: false })
    } else if state.is_some_and(|state| state.running.is_some()) {
        status
            .and_then(|status| status.container_id.as_deref())
            .map(|id| Served::Instance { id: id.into(), running: true })
    } else {
        ended.and_then(id).map(|id| Served::Instance { id, running: false })
    };
    let served = served.unwrap_or_else(|| Served::Nothing(not_started(pod, state, previous)));
    let end = ended.map(end_of);
    Some(Run { served, starts_again: starts_again(pod, name, ended), end })
}

/// Why a container has no log.
fn not_started(pod: &Pod, state: Option<&ContainerState>, previous: bool) -> String {
    if previous {
        return "the container has no earlier instance".to_owned();
    }
    if let Some(waiting) = state.and_then(|state| state.waiting.as_ref()) {
        let reason = waiting.reason.as_deref().unwrap_or("no reason");
        return match waiting.message.as_deref().filter(|message| !message.is_empty()) {
            Some(message) => format!("the container did not start: {reason}: {message}"),
            None => format!("the container did not start: {reason}"),
        };
    }
    let status = pod.status.as_ref();
    let unscheduled = status
        .and_then(|status| status.conditions.as_ref())
        .into_iter()
        .flatten()
        .find(|condition| condition.type_ == "PodScheduled" && condition.status == "False");
    if let Some(condition) = unscheduled {
        let why = condition.message.as_deref().or(condition.reason.as_deref()).unwrap_or("no reason");
        return format!("the pod has no node: {why}");
    }
    let phase = status.and_then(|status| status.phase.as_deref()).unwrap_or("Pending");
    format!("the container did not start: the pod is {phase}")
}

/// True when the pod starts the container again after the end `ended`.
fn starts_again(pod: &Pod, name: &str, ended: Option<&ContainerStateTerminated>) -> bool {
    let status = pod.status.as_ref();
    let phase = status.and_then(|status| status.phase.as_deref());
    if matches!(phase, Some("Succeeded" | "Failed")) || pod.metadata.deletion_timestamp.is_some() {
        return false;
    }
    let spec = pod.spec.as_ref();
    let policy = spec.and_then(|spec| spec.restart_policy.as_deref()).unwrap_or("Always");
    let failed = ended.is_some_and(|run| run.exit_code != 0);
    let init = spec.and_then(|spec| spec.init_containers.as_ref()).into_iter().flatten().find(|init| init.name == name);
    match init {
        // A sidecar runs for the life of the pod.
        Some(init) if init.restart_policy.as_deref() == Some("Always") => true,
        Some(_) => failed && policy != "Never",
        None => policy == "Always" || (policy == "OnFailure" && failed),
    }
}

/// The status of each container of a pod, also of the init and ephemeral containers.
fn statuses(pod: &Pod) -> impl Iterator<Item = &ContainerStatus> {
    let status = pod.status.as_ref();
    let lists = status.into_iter().flat_map(|status| {
        [&status.init_container_statuses, &status.container_statuses, &status.ephemeral_container_statuses]
    });
    lists.flatten().flatten()
}

fn terminated(state: Option<&ContainerState>) -> Option<&ContainerStateTerminated> {
    state?.terminated.as_ref()
}

fn has_id(run: &ContainerStateTerminated) -> bool {
    run.container_id.as_deref().is_some_and(|id| !id.is_empty())
}

/// "Error, exit code 1": the end of an instance.
fn end_of(run: &ContainerStateTerminated) -> String {
    let reason = run.reason.as_deref().filter(|reason| !reason.is_empty()).unwrap_or("Ended");
    format!("{reason}, exit code {}", run.exit_code)
}

#[cfg(test)]
mod tests;
