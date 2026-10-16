//! The steps before the stream: the pod of the request in its store, and a new container of
//! the app, the pod of a node shell or a debug container, up to its start.
//!
//! A write goes out one time (design rule 6). After a write whose result is not known, the
//! step reads the pod: the server tells whether the container or the pod exists.

use std::sync::Arc;
use std::time::Duration;

use fresnel_net::ErrorClass;
use k8s_openapi::api::core::v1::{ContainerState, ContainerStatus, Pod};
use tokio::time::{Instant, timeout_at};

use super::Parts;
use crate::cluster::Fate;
use crate::error::CoreError;
use crate::kinds::Kind;
use crate::ops::{Target, shell};
use crate::store::{ObjectKey, Snapshot};
use crate::terminal::{SessionState, Terminal, TerminalRequest};

/// The longest wait for the start of a new container: the pull of its image and its start.
const START_LIMIT: Duration = Duration::from_secs(180);
/// The reasons of a waiting container that end the wait at once: no try of the kubelet
/// helps, and the user sees the reason at once and not after the limit.
const HOPELESS: [&str; 6] = [
    "ErrImagePull",
    "ImagePullBackOff",
    "InvalidImageName",
    "CreateContainerConfigError",
    "CreateContainerError",
    "RunContainerError",
];

/// The pod of a request in its store, when the store knows it: a pod that is gone, an other
/// pod of the name, a pod that does not run and a container that the pod lacks are errors.
/// A store with no complete list cannot tell, and the server decides.
pub(super) fn pod_of(parts: &Parts, request: &TerminalRequest) -> Result<Option<Arc<Pod>>, CoreError> {
    let pods = parts.pods.read();
    let Some(pod) = pods.items.get(&request.object).cloned() else {
        return if pods.listed { Err(CoreError::invalid("the pod is gone")) } else { Ok(None) };
    };
    if pod.metadata.uid.as_deref() != Some(&*request.uid) {
        return Err(CoreError::invalid("a new pod with the same name took its place"));
    }
    let phase = pod.status.as_ref().and_then(|status| status.phase.as_deref()).unwrap_or("Pending");
    if phase != "Running" {
        return Err(CoreError::invalid(format!("the pod is {phase}, not Running")));
    }
    if spec_flags(&pod, &request.container).is_none() {
        return Err(CoreError::invalid(format!("the pod has no container {}", request.container)));
    }
    Ok(Some(pod))
}

/// The input and the terminal of a container in its spec, for an attach: the server sends
/// no output of a terminal that the container does not have. `None` for no such container.
pub(super) fn spec_flags(pod: &Pod, container: &str) -> Option<(bool, bool)> {
    let spec = pod.spec.as_ref()?;
    let main = spec.containers.iter().chain(spec.init_containers.iter().flatten());
    let found = main.map(|c| (c.name.as_str(), c.stdin, c.tty));
    let ephemeral = spec.ephemeral_containers.iter().flatten().map(|c| (c.name.as_str(), c.stdin, c.tty));
    let (_, stdin, tty) = found.chain(ephemeral).find(|(name, ..)| *name == container)?;
    Some((stdin.unwrap_or(false), tty.unwrap_or(false)))
}

/// Adds a debug container to the pod of the request and waits for its start.
pub(super) async fn add_debug(
    parts: &Parts,
    (request, terminal): (&TerminalRequest, &Terminal),
    name: &str,
    command: &[String],
) -> Result<(), CoreError> {
    let what = format!("the debug container {name}");
    terminal.set_state(SessionState::Starting { step: format!("{what} is added to the pod") });
    let pod = Target {
        cluster: request.cluster.clone(),
        kind: Kind::Pod,
        key: request.object.clone(),
        uid: Arc::clone(&request.uid),
    };
    let container = shell::debug_container(name, &parts.launch.debug_image, &request.container, command);
    let sent = parts.gateway.send::<Pod>(shell::add_debug_container(&pod, &container)?).await;
    if let Err(error) = sent {
        if error.fate != Fate::Unknown {
            return Err(error.error);
        }
        let read: Pod = parts.gateway.get(shell::read_pod(&pod.key.namespace, &pod.key.name)?).await?;
        let ephemeral = read.spec.and_then(|spec| spec.ephemeral_containers).unwrap_or_default();
        if !ephemeral.iter().any(|container| container.name == name) {
            return Err(error.error);
        }
    }
    until_running(parts, terminal, (&request.object, &request.uid), name, &what).await
}

/// Creates the pod of a node shell in the namespace of the settings. Gives the pod as a
/// target, with the UID of the server, for its wait and its delete.
pub(super) async fn create_node_pod(
    parts: &Parts,
    (request, terminal): (&TerminalRequest, &Terminal),
    command: &[String],
) -> Result<Target, CoreError> {
    let (namespace, node) = (&parts.launch.node_namespace, &request.object.name);
    let name = shell::node_shell_name(node);
    terminal.set_state(SessionState::Starting { step: format!("the pod {namespace}/{name} is created") });
    let manifest = shell::node_shell_pod(&name, node, &parts.launch.node_image, command);
    let created = match parts.gateway.send::<Pod>(shell::create_pod(namespace, &manifest)?).await {
        Ok(pod) => pod,
        Err(error) if error.fate == Fate::Unknown => {
            let read = parts.gateway.get::<Pod>(shell::read_pod(namespace, &name)?).await;
            read.map_err(|read| if read.status() == Some(404) { error.error } else { read })?
        }
        Err(error) => return Err(error.error),
    };
    let uid = created.metadata.uid.ok_or_else(|| CoreError::invalid("the server gave the pod no UID"))?;
    let key = ObjectKey::new(namespace.as_str(), name.as_str());
    Ok(Target { cluster: request.cluster.clone(), kind: Kind::Pod, key, uid: uid.into() })
}

/// Waits until the container of the pod with this UID runs, with the state of each step on
/// the line of the terminal. The store of the pods follows each namespace.
pub(super) async fn until_running(
    parts: &Parts,
    terminal: &Terminal,
    (key, uid): (&ObjectKey, &str),
    container: &str,
    what: &str,
) -> Result<(), CoreError> {
    let deadline = Instant::now() + START_LIMIT;
    let mut pods = parts.pods.clone();
    loop {
        let reason = match look(&pods.read(), (key, uid), container) {
            Look::Runs => return Ok(()),
            Look::Fails(reason) => return Err(CoreError::invalid(format!("{what} did not start: {reason}"))),
            Look::Waits(reason) => reason,
        };
        terminal.set_state(SessionState::Starting { step: format!("{what} starts: {reason}") });
        (parts.wake)();
        if timeout_at(deadline, pods.changed()).await.is_err() {
            let limit = START_LIMIT.as_secs();
            return Err(CoreError::new(ErrorClass::Timeout, format!("{what} did not run in {limit} s: {reason}")));
        }
    }
}

/// A new container of the app in the store of the pods.
#[derive(Debug, PartialEq, Eq)]
enum Look {
    Runs,
    Waits(String),
    Fails(String),
}

fn look(pods: &Snapshot<Pod>, (key, uid): (&ObjectKey, &str), container: &str) -> Look {
    let Some(pod) = pods.items.get(key) else {
        return Look::Waits("the pod is not in the store yet".to_owned());
    };
    if pod.metadata.uid.as_deref() != Some(uid) {
        return Look::Fails("the pod is gone".to_owned());
    }
    let status = pod.status.as_ref();
    if let Some(phase @ ("Failed" | "Succeeded")) = status.and_then(|status| status.phase.as_deref()) {
        return Look::Fails(format!("the pod ended ({phase})"));
    }
    let statuses = status.into_iter().flat_map(|status| {
        let main = status.container_statuses.iter().flatten();
        main.chain(status.ephemeral_container_statuses.iter().flatten())
    });
    let found: Option<&ContainerStatus> = statuses.into_iter().find(|status| status.name == container);
    match found.and_then(|status| status.state.as_ref()) {
        None => Look::Waits("the kubelet did not report the container yet".to_owned()),
        Some(ContainerState { running: Some(_), .. }) => Look::Runs,
        Some(ContainerState { terminated: Some(ended), .. }) => {
            let reason = ended.reason.as_deref().unwrap_or("no reason");
            Look::Fails(format!("the container ended: {reason}, exit code {}", ended.exit_code))
        }
        Some(ContainerState { waiting: Some(waiting), .. }) => {
            let reason = waiting.reason.as_deref().unwrap_or("waiting");
            let text = match waiting.message.as_deref() {
                Some(message) => format!("{reason}: {message}"),
                None => reason.to_owned(),
            };
            if HOPELESS.contains(&reason) { Look::Fails(text) } else { Look::Waits(text) }
        }
        Some(_) => Look::Waits("the container has no state yet".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use k8s_openapi::api::core::v1::{Container, ContainerStatus, PodSpec};
    use serde_json::json;

    use super::*;

    fn pod(uid: &str, phase: &str) -> Pod {
        let container =
            Container { name: "app".to_owned(), stdin: Some(true), tty: Some(true), ..Container::default() };
        serde_json::from_value(json!({
            "metadata": {"name": "web-1", "namespace": "shop", "uid": uid},
            "spec": serde_json::to_value(PodSpec { containers: vec![container], ..PodSpec::default() }).unwrap(),
            "status": {"phase": phase},
        }))
        .expect("a pod")
    }

    #[test]
    fn the_spec_flags_give_the_input_and_the_terminal_of_a_container_for_an_attach() {
        let pod = pod("u", "Running");
        assert_eq!(spec_flags(&pod, "app"), Some((true, true)));
        assert_eq!(spec_flags(&pod, "other"), None, "no such container");
    }

    /// A snapshot of the pods with one pod, for the `look` tests.
    fn snapshot_of(pod: Pod) -> Snapshot<Pod> {
        let mut snapshot = Snapshot::<Pod>::empty(crate::store::StoreState::Live, std::time::SystemTime::UNIX_EPOCH);
        snapshot.items = std::iter::once(Arc::new(pod)).collect();
        snapshot
    }

    fn with_debug(state: serde_json::Value) -> Pod {
        let mut pod = pod("u", "Running");
        let status = ContainerStatus {
            name: "debug".to_owned(),
            state: serde_json::from_value(state).ok(),
            ..Default::default()
        };
        pod.status.as_mut().unwrap().ephemeral_container_statuses = Some(vec![status]);
        pod
    }

    #[test]
    fn the_look_tells_a_running_container_a_waiting_one_and_a_hopeless_one() {
        let key = ObjectKey::new("shop", "web-1");
        let empty = Snapshot::<Pod>::empty(crate::store::StoreState::Live, std::time::SystemTime::UNIX_EPOCH);
        assert_eq!(look(&empty, (&key, "u"), "debug"), Look::Waits("the pod is not in the store yet".to_owned()));
        let running = snapshot_of(with_debug(json!({"running": {"startedAt": "2027-01-01T00:00:00Z"}})));
        assert_eq!(look(&running, (&key, "u"), "debug"), Look::Runs);
        assert_eq!(look(&running, (&key, "other"), "debug"), Look::Fails("the pod is gone".to_owned()));
        let waiting = |reason: &str| {
            let snapshot = snapshot_of(with_debug(json!({"waiting": {"reason": reason, "message": "later"}})));
            look(&snapshot, (&key, "u"), "debug")
        };
        assert_eq!(waiting("ContainerCreating"), Look::Waits("ContainerCreating: later".to_owned()));
        assert_eq!(
            waiting("ErrImagePull"),
            Look::Fails("ErrImagePull: later".to_owned()),
            "no try of the kubelet helps"
        );
    }
}
