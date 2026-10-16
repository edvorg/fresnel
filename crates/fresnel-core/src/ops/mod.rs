//! The operations on objects (plan section 10): one module for all of them.
//!
//! Each request has one function that builds it, and a unit test asserts its URL and its
//! body. The semantics are the ones of kubectl; `research/08-operations-semantics.md` of the
//! plan gives each step. An [`Order`] is one operation on one object of one cluster, and a
//! procedure runs it through the gateway of that cluster as a job with steps.
//!
//! - `request`: the paths, the patches and the field manager. `workload`: scale, restart,
//!   suspend and resume. `delete`: delete, force delete and evict. `node`: cordon and the
//!   pods on a node. `drain`: the options and the verdicts of a drain.
//! - `rollout`: the rollout status of kubectl. `history`: the revisions of a workload.
//!   `rollback`: the patch that goes back to a revision.
//! - `job`: run a Job again, and the Job of a CronJob that the user triggers.
//! - `document`: the create and the server-side apply of a document of a text.
//! - `kubectl`: the `kubectl` commands that a user can copy. `shell`: the pod of a node
//!   shell and a debug container of a terminal.
//! - `state`: the state of a job. `procedure`: the steps of each operation.
//! - `access`: the permissions of the user in a cluster.

pub mod access;
mod delete;
mod document;
pub mod drain;
mod history;
mod job;
pub mod kubectl;
mod node;
mod procedure;
mod request;
mod rollback;
mod rollout;
pub(crate) mod shell;
mod state;
mod workload;

use std::sync::Arc;

use drain::DrainOptions;
pub use history::Revision;
pub use job::trigger_name;
pub use procedure::{After, Cancel, Parts, Progress, run};
pub use rollout::Rollout;
pub use state::{JobState, Outcome, Report, Step, StepState};

use crate::cluster::ClusterId;
use crate::error::CoreError;
use crate::kinds::{Action, Kind};
use crate::manifest::documents::Document;
use crate::store::ObjectKey;

/// The object of an operation.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Target {
    /// The cluster of the object. A request goes to this cluster only (design rule 8).
    pub cluster: ClusterId,
    /// Its kind.
    pub kind: Kind,
    /// Its namespace and name.
    pub key: ObjectKey,
    /// The UID of the object that the user sees. A delete sends it as a precondition: an
    /// object of the same name can be a new object (research 08 section 3.7).
    pub uid: Arc<str>,
}

/// How a delete treats the objects that the object owns (research 08 section 4.8).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Propagation {
    /// The object goes at once, and the garbage collector deletes its dependents later. The
    /// default of kubectl.
    Background,
    /// The object stays until each dependent is gone.
    Foreground,
    /// The dependents stay, with no owner.
    Orphan,
}

/// One operation. The kinds that have it are in [`Operation::fits`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Operation {
    /// Sets the count of pods through the subresource `scale`.
    Scale(i32),
    /// Starts each pod again, as `kubectl rollout restart` does.
    Restart,
    /// Follows the rollout until it completes, fails or the user cancels it.
    RolloutStatus,
    /// Reads the revisions of the pod template.
    History,
    /// Goes back to a revision. 0 is the revision before the current one, as in kubectl.
    Rollback(i64),
    /// Deletes the object with this propagation policy.
    Delete(Propagation),
    /// Deletes the object with no grace period: the server does not wait for the kubelet.
    ForceDelete,
    /// Evicts a pod through the eviction API, which obeys each PodDisruptionBudget.
    Evict,
    /// Deletes a Job and creates it again from its manifest.
    RunAgain,
    /// Stops a Job or a CronJob.
    Suspend,
    /// Ends the suspend of a Job or a CronJob.
    Resume,
    /// Creates a Job with this name from a CronJob now.
    Trigger(String),
    /// Closes a node for new pods.
    Cordon,
    /// Opens a node for new pods again.
    Uncordon,
    /// Cordons a node and evicts its pods with these options, as `kubectl drain`.
    Drain(DrainOptions),
    /// Saves the manifest of the object as a new YAML file in the folder of downloads.
    Download,
    /// Creates the object of a document of a text (plan section 12.9).
    Create(Document),
    /// Applies a document of a text with server-side apply. A force takes the fields that
    /// an other field manager set.
    Apply { document: Document, force: bool },
}

impl Operation {
    /// The action of the command table that starts the operation on an object. `None` for
    /// the create and the apply of a text, which start from no object.
    pub fn action(&self) -> Option<Action> {
        Some(match self {
            Self::Scale(_) => Action::Scale,
            Self::Restart => Action::Restart,
            Self::RolloutStatus => Action::RolloutStatus,
            Self::History => Action::RevisionHistory,
            Self::Rollback(_) => Action::Rollback,
            Self::Delete(_) => Action::Delete,
            Self::ForceDelete => Action::ForceDelete,
            Self::Evict => Action::Evict,
            Self::RunAgain => Action::RunAgain,
            Self::Suspend => Action::Suspend,
            Self::Resume => Action::Resume,
            Self::Trigger(_) => Action::Trigger,
            Self::Cordon => Action::Cordon,
            Self::Uncordon => Action::Uncordon,
            Self::Drain(_) => Action::Drain,
            Self::Download => Action::Download,
            Self::Create(_) | Self::Apply { .. } => return None,
        })
    }

    /// False for an operation that only reads: a stop of its job changes nothing.
    pub fn writes(&self) -> bool {
        !matches!(self, Self::RolloutStatus | Self::History | Self::Download)
    }

    /// True when a job of `other` on the same object does the same work: a second drain of
    /// a node with other options would race the first one for its pods.
    pub fn same_work(&self, other: &Self) -> bool {
        self == other || matches!((self, other), (Self::Drain(_), Self::Drain(_)))
    }

    /// True when objects of the kind have the operation: the kind lists its action. A
    /// Deployment has no suspend, so the request would change nothing or fail. A document
    /// fits the kind that it names.
    pub fn fits(&self, kind: Kind) -> bool {
        match (self, self.action()) {
            (Self::Create(document) | Self::Apply { document, .. }, _) => document.kind == kind,
            (_, Some(action)) => kind.actions().any(|of| of == action),
            (_, None) => false,
        }
    }

    /// The words of the operation for a list of jobs.
    pub fn label(&self) -> String {
        match self {
            Self::Scale(replicas) => format!("Scale to {replicas}"),
            Self::Rollback(0) => "Roll back to the revision before".to_owned(),
            Self::Rollback(revision) => format!("Roll back to revision {revision}"),
            Self::Delete(Propagation::Background) => "Delete".to_owned(),
            Self::Delete(Propagation::Foreground) => "Delete in the foreground".to_owned(),
            Self::Delete(Propagation::Orphan) => "Delete and keep the dependents".to_owned(),
            Self::Trigger(name) => format!("Trigger now as {name}"),
            Self::Drain(options) if *options != DrainOptions::default() => format!("Drain {}", options.flags()),
            Self::Create(_) => "Create".to_owned(),
            Self::Apply { force: false, .. } => "Apply".to_owned(),
            Self::Apply { force: true, .. } => "Apply with force".to_owned(),
            other => other.action().map_or("", Action::label).to_owned(),
        }
    }
}

/// One operation on one object.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Order {
    /// The object.
    pub target: Target,
    /// What to do with it.
    pub operation: Operation,
}

impl Order {
    /// The order, when the kind of the object has the operation.
    pub fn new(target: Target, operation: Operation) -> Result<Self, CoreError> {
        if !operation.fits(target.kind) {
            let kind = target.kind.info().name;
            return Err(CoreError::invalid(format!("a {kind} has no operation \"{}\"", operation.label())));
        }
        Ok(Self { target, operation })
    }

    /// The orders of a bulk action on the selected rows (plan section 10): one for each
    /// object, so that each one gets its own job and its own state. An object whose kind
    /// does not have the operation is an error, and no order goes.
    pub fn bulk(targets: impl IntoIterator<Item = Target>, operation: &Operation) -> Result<Vec<Self>, CoreError> {
        targets.into_iter().map(|target| Self::new(target, operation.clone())).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(kind: Kind, name: &str) -> Target {
        Target { cluster: ClusterId::new("a"), kind, key: ObjectKey::new("shop", name), uid: "uid".into() }
    }

    #[test]
    fn an_order_needs_a_kind_with_its_action_and_a_bulk_action_makes_one_order_for_each_row() {
        assert!(Order::new(target(Kind::Deployment, "web"), Operation::Scale(3)).is_ok());
        let error = Order::new(target(Kind::DaemonSet, "agent"), Operation::Scale(3)).expect_err("no scale");
        assert_eq!(error.to_string(), "a DaemonSet has no operation \"Scale to 3\"");
        assert!(Operation::Delete(Propagation::Background).fits(Kind::Namespace), "each kind has delete");
        assert!(!Operation::Evict.fits(Kind::Deployment));
        assert!(Operation::Suspend.fits(Kind::Job) && Operation::Suspend.fits(Kind::CronJob));
        let drain = Operation::Drain(drain::DrainOptions { force: true, ..Default::default() });
        assert!(drain.fits(Kind::Node) && !drain.fits(Kind::Pod) && Operation::Cordon.fits(Kind::Node));
        assert_eq!(
            (drain.label(), Operation::Drain(Default::default()).label()),
            ("Drain --force".to_owned(), "Drain".to_owned())
        );
        assert!(drain.same_work(&Operation::Drain(Default::default())) && !drain.same_work(&Operation::Cordon));

        let rows = [target(Kind::Deployment, "web"), target(Kind::StatefulSet, "db")];
        let orders = Order::bulk(rows.clone(), &Operation::Restart).expect("both restart");
        assert_eq!(orders.iter().map(|order| &*order.target.key.name).collect::<Vec<_>>(), ["web", "db"]);
        let mixed = [target(Kind::Deployment, "web"), target(Kind::Job, "once")];
        assert!(Order::bulk(mixed, &Operation::Restart).is_err(), "a Job has no restart: no order goes");
    }
}
