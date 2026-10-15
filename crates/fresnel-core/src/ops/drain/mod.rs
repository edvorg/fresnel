//! The drain of a node (plan section 10, research 08 section 7.2): the options of kubectl
//! and the verdict of each pod on the node, by the filters of kubectl in their sequence.
//!
//! The dialog of a drain and its job use the same verdicts: the dialog reads the stores, and
//! the job a list and the DaemonSets of the server after its cordon. A pod that blocks
//! stops the drain before its first eviction, as in kubectl.
//!
//! - `report`: the state of each pod while the drain runs, and what an answer to an
//!   eviction means for the next step.

mod report;

use std::time::Duration;

use k8s_openapi::api::core::v1::Pod;

pub use report::{Drain, DrainPod, PodState};
pub(super) use report::{InStore, Next, RETRY, after};

use crate::store::ObjectKey;

/// The annotation of a mirror pod: the kubelet runs it from a file of the node, and the API
/// holds only a copy, which an eviction would not stop.
const MIRROR: &str = "kubernetes.io/config.mirror";
/// The most pods that the text of a refusal names; the dialog and the report list each one.
const NAMED: usize = 5;

/// The options of a drain. The default is the default of kubectl.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct DrainOptions {
    /// `--ignore-daemonsets`: a pod of a DaemonSet stays on the node. Its DaemonSet tolerates
    /// the cordon and would start it again at once.
    pub ignore_daemon_sets: bool,
    /// `--delete-emptydir-data`: a pod with an `emptyDir` volume goes, with the data in it.
    pub delete_empty_dir_data: bool,
    /// `--force`: a pod with no controller goes, and nothing starts it again.
    pub force: bool,
    /// `--grace-period`, in seconds, for each pod. `None`: the grace period of each pod.
    pub grace_period: Option<u32>,
    /// `--timeout`: the drain stops after this time of its evictions. `None`: no limit.
    pub timeout: Option<Duration>,
}

impl DrainOptions {
    /// The options as the flags of kubectl, for the label of a job: "--force --timeout=60s".
    pub fn flags(&self) -> String {
        let mut flags = Vec::new();
        let set = [
            (self.ignore_daemon_sets, "--ignore-daemonsets"),
            (self.delete_empty_dir_data, "--delete-emptydir-data"),
            (self.force, "--force"),
        ];
        flags.extend(set.iter().filter(|(on, _)| *on).map(|(_, flag)| (*flag).to_owned()));
        flags.extend(self.grace_period.map(|grace| format!("--grace-period={grace}")));
        flags.extend(self.timeout.map(|timeout| format!("--timeout={}s", timeout.as_secs())));
        flags.join(" ")
    }
}

/// What the drain does with one pod.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The drain evicts the pod. The text warns of what goes with it.
    Evict(Option<&'static str>),
    /// The pod stays on the node, and the drain goes on. The text tells why.
    Keep(&'static str),
    /// The pod stops the drain before its first eviction.
    Block(Block),
}

/// Why a pod stops a drain, and the option that removes the reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    /// A pod of a DaemonSet, with no option "ignore DaemonSets".
    DaemonSet,
    /// A pod with an `emptyDir` volume, with no option "delete emptyDir data".
    LocalData,
    /// A pod with no controller, with no option "force".
    NoController,
    /// A pod whose DaemonSet, of this name, is not there, with no option "force".
    NoDaemonSet(String),
    /// The read of the DaemonSet of the pod failed. No option removes this.
    Unread(String),
}

impl Block {
    /// The reason and the option that removes it, in the words of the dialog.
    pub fn text(&self) -> String {
        match self {
            Self::DaemonSet => "a pod of a DaemonSet: \"ignore DaemonSets\" keeps it on the node".to_owned(),
            Self::LocalData => "a pod with emptyDir data: \"delete emptyDir data\" evicts it".to_owned(),
            Self::NoController => {
                "a pod with no controller: \"force\" evicts it, and nothing starts it again".to_owned()
            }
            Self::NoDaemonSet(name) => format!("its DaemonSet {name} is not there: \"force\" evicts it"),
            Self::Unread(error) => format!("the read of its DaemonSet failed: {error}"),
        }
    }
}

/// What the app knows of the DaemonSet of a pod.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Found {
    /// It exists.
    Here,
    /// It is not there: the server said 404, or a store with a complete list has none.
    Missing,
    /// The app cannot tell: the text tells why.
    Unknown(String),
}

/// The verdict of a pod by the filters of kubectl, in their sequence: DaemonSet, mirror
/// pod, `emptyDir` data, controller. `daemon_set` finds the DaemonSet of this name in the
/// namespace of the pod. A pod that ended passes each filter: its eviction stops nothing.
pub fn verdict(pod: &Pod, options: &DrainOptions, daemon_set: impl FnOnce(&str) -> Found) -> Verdict {
    let phase = pod.status.as_ref().and_then(|status| status.phase.as_deref());
    let ended = matches!(phase, Some("Succeeded" | "Failed"));
    let owners = pod.metadata.owner_references.iter().flatten();
    let controller = owners.into_iter().find(|owner| owner.controller == Some(true));
    let mut warning = None;
    if let Some(owner) = controller.filter(|owner| owner.kind == "DaemonSet" && !ended) {
        match daemon_set(&owner.name) {
            Found::Here if options.ignore_daemon_sets => return Verdict::Keep("a pod of a DaemonSet"),
            Found::Here => return Verdict::Block(Block::DaemonSet),
            Found::Missing if options.force => warning = Some("its DaemonSet is not there"),
            Found::Missing => return Verdict::Block(Block::NoDaemonSet(owner.name.clone())),
            Found::Unknown(reason) => return Verdict::Block(Block::Unread(reason)),
        }
    }
    let annotations = pod.metadata.annotations.as_ref();
    if annotations.is_some_and(|annotations| annotations.contains_key(MIRROR)) {
        return Verdict::Keep("a mirror pod: the kubelet runs it from a file of the node");
    }
    let volumes = pod.spec.as_ref().and_then(|spec| spec.volumes.as_ref());
    if !ended && volumes.into_iter().flatten().any(|volume| volume.empty_dir.is_some()) {
        if !options.delete_empty_dir_data {
            return Verdict::Block(Block::LocalData);
        }
        warning = Some("its emptyDir data goes with it");
    }
    if !ended && controller.is_none() {
        if !options.force {
            return Verdict::Block(Block::NoController);
        }
        warning = Some("it has no controller: nothing starts it again");
    }
    Verdict::Evict(warning)
}

/// The text of a drain that pods block: the count, and the first pods with their reasons.
pub(crate) fn blocked(pods: &[(&ObjectKey, &Block)]) -> String {
    let named: Vec<String> = pods
        .iter()
        .take(NAMED)
        .map(|(key, block)| format!("{}/{}: {}", key.namespace, key.name, block.text()))
        .collect();
    let more = pods.len().saturating_sub(NAMED);
    let more = if more > 0 { format!("; and {more} more") } else { String::new() };
    let count = if pods.len() == 1 { "1 pod blocks".to_owned() } else { format!("{} pods block", pods.len()) };
    format!("{count} the drain, so no eviction went out. {}{more}", named.join("; "))
}

#[cfg(test)]
mod tests;
