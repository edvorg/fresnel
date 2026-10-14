//! The actions on objects (plan sections 10 and 11.5): what the user asks for, whether the
//! object and the permissions of the user allow it, and the dialog that asks before a change.
//!
//! Each action comes from the one command table of the core. The header of the panel, the
//! menu of a row, a key, the quick search and the bar of a bulk action ask for an action
//! here; an action that changes a cluster opens the confirm dialog first, and its end is
//! `Command::Run` with one order for each object.
//!
//! - `facts`: what an action reads of its object. `permits`: the permissions of the user.
//! - `dialog`: the dialog of an action: `confirm` before a change, `commands` for kubectl.

mod commands;
mod confirm;
mod dialog;
pub(crate) mod facts;
mod permits;

use std::sync::Arc;

use fresnel_core::commands::{self as table, CommandId, Confirm};
use fresnel_core::kinds::{Action, Describe};
use fresnel_core::ops::access::Decision;
use fresnel_core::ops::{Operation, Propagation, Target};

pub(crate) use commands::Commands;
pub(crate) use confirm::Confirm as ConfirmDialog;
pub(crate) use dialog::{Close, Dialog};
pub(crate) use facts::Facts;
pub(crate) use permits::Permits;

use crate::views::detail::actions::{can_run, later};
use crate::widgets::table::model::ObjectId;

/// An action that the user asked for on the object of the panel, with the choice of a part
/// of the panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Ask {
    pub(crate) action: Action,
    /// The revision of a rollback that the user chose in the list of the revisions. `None`:
    /// the revision before the current one.
    pub(crate) revision: Option<i64>,
}

/// One object of an action: its identity and what the action reads of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Subject {
    pub(crate) target: Target,
    pub(crate) facts: Facts,
}

impl Subject {
    pub(crate) fn of<K: Describe>(id: &ObjectId, object: &K) -> Self {
        let target = Target { cluster: id.cluster.clone(), kind: K::ID, key: id.key.clone(), uid: Arc::clone(&id.uid) };
        Self { target, facts: facts::of(object) }
    }

    /// "Deployment shop/web in ams-prod": the object in a note.
    pub(crate) fn text(&self) -> String {
        let Target { cluster, kind, key, .. } = &self.target;
        let object =
            if key.namespace.is_empty() { key.name.to_string() } else { format!("{}/{}", key.namespace, key.name) };
        format!("{} {object} in {cluster}", kind.info().name)
    }
}

/// Whether an action can run on an object now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Gate {
    /// The rules of the user permit it.
    Open,
    /// The app does not know the permission yet. The action runs, and the server decides.
    Unknown(String),
    /// The action cannot run: the text tells why.
    Closed(String),
}

impl Gate {
    /// The gate of an action on an object with these facts. `decide` gives the permission:
    /// it asks the core for what it does not know, so it runs only for an action that runs.
    pub(crate) fn of(action: Action, facts: &Facts, decide: impl FnOnce() -> Decision) -> Self {
        if !can_run(action) {
            return Self::Closed(later(action));
        }
        if let Some(reason) = blocked(action, facts) {
            return Self::Closed(reason);
        }
        match decide() {
            Decision::Allowed => Self::Open,
            Decision::Denied(reason) => Self::Closed(format!("Forbidden: {reason}.")),
            Decision::Unknown(reason) => Self::Unknown(reason),
        }
    }

    pub(crate) fn is_closed(&self) -> bool {
        matches!(self, Self::Closed(_))
    }

    /// The text for the pointer over a control of the action, if any.
    pub(crate) fn hint(&self) -> Option<String> {
        match self {
            Self::Open => None,
            Self::Unknown(reason) => {
                Some(format!("The app does not know if you may do this ({reason}). The server decides."))
            }
            Self::Closed(reason) => Some(reason.clone()),
        }
    }
}

/// Why the state of the object refuses an action, with the words of kubectl where it has some.
fn blocked(action: Action, facts: &Facts) -> Option<String> {
    match action {
        Action::Restart if facts.paused => {
            Some("can't restart paused deployment (run rollout resume first)".to_owned())
        }
        Action::Rollback if facts.paused => Some("you cannot rollback a paused deployment".to_owned()),
        Action::Suspend if facts.suspended == Some(true) => Some("It is suspended already.".to_owned()),
        Action::Resume if facts.suspended == Some(false) => Some("It is not suspended.".to_owned()),
        _ => None,
    }
}

/// True for an action that asks before it acts, by its entry in the command table.
pub(crate) fn asks_first(action: Action) -> bool {
    table::entry(CommandId::Object(action)).is_none_or(|entry| entry.confirm == Confirm::Always)
}

/// The operation of an action that needs no input: the read actions, and the default of
/// the dialog of each other one.
pub(crate) fn operation(action: Action) -> Option<Operation> {
    Some(match action {
        Action::Restart => Operation::Restart,
        Action::RolloutStatus => Operation::RolloutStatus,
        Action::RevisionHistory => Operation::History,
        Action::Rollback => Operation::Rollback(0),
        Action::Delete => Operation::Delete(Propagation::Background),
        Action::ForceDelete => Operation::ForceDelete,
        Action::Evict => Operation::Evict,
        Action::RunAgain => Operation::RunAgain,
        Action::Suspend => Operation::Suspend,
        Action::Resume => Operation::Resume,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use fresnel_core::cluster::ClusterId;
    use fresnel_core::store::ObjectKey;

    use super::*;

    #[test]
    fn a_gate_closes_for_a_later_milestone_a_state_and_a_denial_and_stays_open_when_unknown() {
        let facts = Facts { paused: true, suspended: Some(false), ..Facts::default() };
        let gate = |action| Gate::of(action, &facts, || Decision::Allowed);
        assert!(matches!(gate(Action::Logs), Gate::Closed(reason) if reason.contains("later milestone")));
        assert_eq!(
            gate(Action::Restart),
            Gate::Closed("can't restart paused deployment (run rollout resume first)".to_owned())
        );
        assert_eq!(gate(Action::Resume), Gate::Closed("It is not suspended.".to_owned()));
        assert_eq!(gate(Action::Suspend), Gate::Open);
        let denied = Gate::of(Action::Delete, &facts, || Decision::Denied("no rule lets you delete pods".to_owned()));
        assert_eq!(denied, Gate::Closed("Forbidden: no rule lets you delete pods.".to_owned()));
        let unknown = Gate::of(Action::Delete, &facts, || Decision::Unknown("the rules are not read yet".to_owned()));
        assert!(!unknown.is_closed());
        assert!(unknown.hint().is_some_and(|hint| hint.contains("The server decides")));
    }

    #[test]
    fn the_command_table_tells_which_action_asks_first() {
        assert!(asks_first(Action::Scale) && asks_first(Action::Delete) && asks_first(Action::Trigger));
        assert!(!asks_first(Action::RolloutStatus) && !asks_first(Action::RevisionHistory));
        let subject = Subject {
            target: Target {
                cluster: ClusterId::new("ams-prod"),
                kind: fresnel_core::kinds::Kind::Deployment,
                key: ObjectKey::new("shop", "web"),
                uid: "u".into(),
            },
            facts: Facts::default(),
        };
        assert_eq!(subject.text(), "Deployment shop/web in ams-prod");
    }
}
