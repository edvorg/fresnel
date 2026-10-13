//! The status text of a pod by the rule of kubectl.
//!
//! The rule is the function `printPod` of Kubernetes 1.33
//! (`pkg/printers/internalversion/printers.go`): the API server runs it for the table of
//! `kubectl get pods`. Each step here is one step of that function, in its sequence, so
//! that a later version of the rule is easy to compare. A test on k3s holds the two equal.

use k8s_openapi::api::core::v1::{
    Container, ContainerState, ContainerStateTerminated, ContainerStatus, Pod, PodCondition,
};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;

/// The restarts of some containers, and the end of the last run before a restart.
#[derive(Clone, Default)]
struct Restarts {
    count: i64,
    last: Option<Time>,
}

impl Restarts {
    fn add(&mut self, container: &ContainerStatus) {
        self.count += i64::from(container.restart_count);
        let ended = terminated(container.last_state.as_ref()).and_then(|run| run.finished_at.as_ref());
        if let Some(ended) = ended.filter(|ended| self.last.as_ref().is_none_or(|last| last < *ended)) {
            self.last = Some(ended.clone());
        }
    }
}

/// What the rule gives for one pod.
pub(super) struct Printed {
    /// The column STATUS.
    pub(super) text: String,
    /// The two numbers of the column READY.
    pub(super) ready: u32,
    pub(super) containers: u32,
    /// The column RESTARTS: the count, and the time that kubectl shows as "ago".
    pub(super) restarts: u32,
    pub(super) last_restart: Option<Time>,
}

fn terminated(state: Option<&ContainerState>) -> Option<&ContainerStateTerminated> {
    state?.terminated.as_ref()
}

/// The reason of a container that waits, when it has one.
fn waiting_reason(state: Option<&ContainerState>) -> Option<&str> {
    state?.waiting.as_ref()?.reason.as_deref().filter(|reason| !reason.is_empty())
}

/// The words for a container that ended: its reason, or its signal, or its exit code.
fn end_of(run: &ContainerStateTerminated) -> String {
    match (run.reason.as_deref().filter(|reason| !reason.is_empty()), run.signal.filter(|signal| *signal != 0)) {
        (Some(reason), _) => reason.to_owned(),
        (None, Some(signal)) => format!("Signal:{signal}"),
        (None, None) => format!("ExitCode:{}", run.exit_code),
    }
}

/// True for an init container that runs for the life of the pod: a "sidecar".
fn is_sidecar(container: &Container) -> bool {
    container.restart_policy.as_deref() == Some("Always")
}

fn condition<'a>(conditions: &'a [PodCondition], name: &str) -> Option<&'a PodCondition> {
    conditions.iter().find(|condition| condition.type_ == name)
}

/// What the init containers add to the row.
struct Init {
    /// The status text of a pod whose init containers are not done.
    waits: Option<String>,
    ready_sidecars: u32,
    /// The restarts of the init containers to the first one that is not done.
    all: Restarts,
    sidecars: Restarts,
}

/// Reads the init containers to the first one that is not done.
fn init(containers: &[Container], statuses: &[ContainerStatus]) -> Init {
    let mut init = Init { waits: None, ready_sidecars: 0, all: Restarts::default(), sidecars: Restarts::default() };
    for (number, status) in statuses.iter().enumerate() {
        init.all.add(status);
        let sidecar = containers.iter().any(|container| container.name == status.name && is_sidecar(container));
        if sidecar {
            init.sidecars.add(status);
        }
        let state = status.state.as_ref();
        let waits = match terminated(state) {
            Some(run) if run.exit_code == 0 => continue,
            _ if sidecar && status.started == Some(true) => {
                init.ready_sidecars += u32::from(status.ready);
                continue;
            }
            Some(run) => format!("Init:{}", end_of(run)),
            None => match waiting_reason(state).filter(|reason| *reason != "PodInitializing") {
                Some(reason) => format!("Init:{reason}"),
                None => format!("Init:{number}/{}", containers.len()),
            },
        };
        init.waits = Some(waits);
        break;
    }
    init
}

/// Reads the containers from the last one to the first one, so that the text is of the
/// first container that waits or ended. Gives the count of ready containers that run.
fn containers(statuses: &[ContainerStatus], restarts: &mut Restarts, text: &mut String) -> u32 {
    let mut running = 0;
    for container in statuses.iter().rev() {
        restarts.add(container);
        let state = container.state.as_ref();
        if let Some(reason) = waiting_reason(state) {
            reason.clone_into(text);
        } else if let Some(run) = terminated(state) {
            *text = end_of(run);
        } else if container.ready && state.is_some_and(|state| state.running.is_some()) {
            running += 1;
        }
    }
    running
}

/// The row of `kubectl get pods` for this pod.
pub(super) fn print(pod: &Pod) -> Printed {
    let (spec, status) = (pod.spec.as_ref(), pod.status.as_ref());
    let phase = status.and_then(|status| status.phase.as_deref()).unwrap_or_default();
    let status_reason = status.and_then(|status| status.reason.as_deref()).filter(|reason| !reason.is_empty());
    let conditions = status.and_then(|status| status.conditions.as_deref()).unwrap_or_default();
    let is_true = |name: &str| condition(conditions, name).is_some_and(|condition| condition.status == "True");
    let init_containers = spec.and_then(|spec| spec.init_containers.as_deref()).unwrap_or_default();
    let init_statuses = status.and_then(|status| status.init_container_statuses.as_deref()).unwrap_or_default();
    let statuses = status.and_then(|status| status.container_statuses.as_deref()).unwrap_or_default();

    let mut text = status_reason.unwrap_or(phase).to_owned();
    if condition(conditions, "PodScheduled").is_some_and(|scheduled| scheduled.reason.as_deref() == Some(GATED)) {
        GATED.clone_into(&mut text);
    }
    let init = init(init_containers, init_statuses);
    let (mut ready, mut restarts) = (init.ready_sidecars, init.all);
    let initializing = init.waits.is_some();
    if let Some(waits) = init.waits {
        text = waits;
    }
    if !initializing || is_true("Initialized") {
        restarts = init.sidecars;
        let running = containers(statuses, &mut restarts, &mut text);
        ready += running;
        // A pod with a container that still runs did not complete.
        if text == "Completed" && running > 0 {
            text = if is_true("Ready") { "Running" } else { "NotReady" }.to_owned();
        }
    }
    if pod.metadata.deletion_timestamp.is_some() {
        if status_reason == Some(NODE_LOST) {
            text = "Unknown".to_owned();
        } else if !matches!(phase, "Failed" | "Succeeded") {
            text = "Terminating".to_owned();
        }
    }

    let sidecars = init_containers.iter().filter(|container| is_sidecar(container)).count();
    let containers = spec.map_or(0, |spec| spec.containers.len()) + sidecars;
    let count = u32::try_from(restarts.count.max(0)).unwrap_or(u32::MAX);
    Printed {
        text,
        ready,
        containers: u32::try_from(containers).unwrap_or(u32::MAX),
        restarts: count,
        // kubectl shows the time only next to a count.
        last_restart: restarts.last.filter(|_| count != 0),
    }
}

/// The reason of the condition `PodScheduled` for a pod with a scheduling gate.
const GATED: &str = "SchedulingGated";
/// The reason of the status of a pod whose node does not answer.
const NODE_LOST: &str = "NodeLost";
