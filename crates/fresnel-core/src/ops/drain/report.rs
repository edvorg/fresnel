//! The state of each pod while a drain runs (research 08 section 7.2: waiting, evicting,
//! blocked by a budget, terminating, gone, failed), and what an answer to an eviction means.

use std::sync::Arc;
use std::time::Duration;

use super::Verdict;
use crate::cluster::{Fate, WriteError};
use crate::error::CoreError;
use crate::store::{ObjectKey, Written};

/// The pause of kubectl after a refused eviction, before the next one.
pub(in crate::ops) const RETRY: Duration = Duration::from_secs(5);
/// The start of the cause of the server for a budget that blocks: the name of the budget
/// follows it.
pub(super) const BUDGET: &str = "The disruption budget ";
/// The longest name of an object. A longer word after [`BUDGET`] is no name of a budget.
pub(super) const NAME_BYTES: usize = 253;
/// The most bytes of the text of a refusal that a pod keeps: the board copies each pod at
/// each change, and the server can send 1 MiB.
pub(super) const REASON_BYTES: usize = 2000;

/// What a drain shows while it runs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Drain {
    /// True after the server stored the cordon. The node then takes no new pod, also
    /// after a cancel or a failure of the drain: kubectl leaves it so, and so does the app.
    pub cordoned: bool,
    /// The answer of the cordon, for the mark "change pending" of the node.
    pub written: Option<Written>,
    /// Each pod on the node, with its verdict and its state, in the sequence of the list.
    pub pods: Arc<[DrainPod]>,
}

/// One pod on the node of a drain.
#[derive(Clone, Debug, PartialEq)]
pub struct DrainPod {
    /// Its namespace and name.
    pub key: ObjectKey,
    /// What the drain does with it.
    pub verdict: Verdict,
    /// How far it is.
    pub state: PodState,
}

/// How far one pod of a drain is.
#[derive(Clone, Debug, PartialEq)]
pub enum PodState {
    /// The pod waits for the evictions, which start after the check of each pod.
    Waiting,
    /// The request of the eviction is on its way.
    Evicting,
    /// The server refused the eviction for now, and the next one goes 5 s later. `budget`
    /// is the PodDisruptionBudget that blocks it, when the server names one; `reason` is
    /// the text of the server; `attempts` counts the refusals.
    Blocked {
        /// The name of the budget.
        budget: Option<String>,
        /// The text of the server.
        reason: String,
        /// The count of refusals.
        attempts: u32,
    },
    /// The server took the eviction, and the pod ends. A drain that stops then leaves the
    /// pod in this state: it goes by itself.
    Terminating,
    /// The pod is gone.
    Gone,
    /// The verdict keeps the pod on the node.
    Kept,
    /// The eviction failed, or the drain cannot follow the pod; the pod can stay.
    Failed(CoreError),
    /// The eviction went out, and no answer came back: the store of the pods shows if it goes.
    Unknown(CoreError),
    /// The drain ended before the server took an eviction of the pod: a cancel, its time
    /// limit, or a pod that blocks.
    Stopped,
}

impl PodState {
    /// The first state of a pod with this verdict.
    pub(in crate::ops) fn first(verdict: &Verdict) -> Self {
        match verdict {
            Verdict::Keep(_) => Self::Kept,
            Verdict::Evict(_) | Verdict::Block(_) => Self::Waiting,
        }
    }
}

/// The next step of the loop of one pod after the answer to its eviction.
#[derive(Debug, PartialEq)]
pub(in crate::ops) enum Next {
    /// The server took the eviction: wait until the pod is gone.
    Wait,
    /// The pod is gone already.
    Gone,
    /// Send the eviction again after the pause, in this state.
    Again(PodState),
    /// The loop ends in this state.
    End(PodState),
}

/// What the pod store shows of the pod of an eviction, at its answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ops) enum InStore {
    /// The pod, with the UID of the list.
    Running,
    /// The pod, with a delete mark: it goes by itself.
    Ending,
    /// No such pod: it is gone, or the store has no complete list yet.
    Unseen,
}

/// What the answer to an eviction means, as in kubectl. `pod`: what the store shows of the
/// pod now. `attempts`: the count of refusals before this one.
///
/// - 404: the pod is gone. 409: the precondition of its UID failed, so a new pod has its
///   name, and the pod of the list is gone (kubectl would evict the new pod). The store tells
///   it: a conflict for a pod that it still shows is a refusal for now.
/// - 429: a budget blocks the eviction for now. 403 with the cause `NamespaceTerminating`:
///   the namespace goes away, and the server takes no eviction there for now.
pub(in crate::ops) fn after(answer: Result<(), WriteError>, pod: InStore, attempts: u32) -> Next {
    let Err(failure) = answer else {
        return Next::Wait;
    };
    let error = &failure.error;
    let again = |budget: Option<&str>, reason: &str| {
        let budget = budget.filter(|name| name.len() <= NAME_BYTES).map(str::to_owned);
        let reason = reason.get(..reason.floor_char_boundary(REASON_BYTES)).unwrap_or_default().to_owned();
        Next::Again(PodState::Blocked { budget, reason, attempts: attempts + 1 })
    };
    // A conflict or a namespace that goes away refuses the eviction for now. A pod with a
    // delete mark then goes by itself, and the loop waits for it.
    let for_now = match error.status() {
        Some(409) => true,
        Some(403) => error.cause("NamespaceTerminating").is_some(),
        _ => false,
    };
    match error.status() {
        Some(404) => Next::Gone,
        Some(409) if pod == InStore::Unseen => Next::Gone,
        Some(429) => {
            let cause = error.cause("DisruptionBudget");
            let budget = cause.and_then(|cause| cause.strip_prefix(BUDGET)?.split_whitespace().next());
            again(budget, cause.unwrap_or(&error.to_string()))
        }
        _ if for_now && pod == InStore::Ending => Next::Wait,
        _ if for_now => again(None, &error.to_string()),
        _ if failure.fate == Fate::Unknown => Next::End(PodState::Unknown(failure.error)),
        _ => Next::End(PodState::Failed(failure.error)),
    }
}
