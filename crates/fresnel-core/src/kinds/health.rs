//! The health value of an object (plan section 11.4): how well it does, as one value.
//!
//! A value comes from the object alone, with no clock and no other object. Thus a row of
//! a table changes its value only when its own object changes. The price: a value cannot
//! tell that a state takes too long. An object on its way is `Progressing` for all time,
//! and only the object itself can say that the way failed, for example with the deadline
//! of a rollout.

use k8s_openapi::api::apps::v1::{DeploymentCondition, ReplicaSetCondition};
use k8s_openapi::api::batch::v1::JobCondition;
use k8s_openapi::api::core::v1::{NamespaceCondition, NodeCondition, PodCondition};

/// How well one object does. The sequence is from good to bad, so the worst of some
/// values is their maximum.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ObjectHealth {
    /// The object is in the state that its spec asks for, or it ended well.
    Healthy,
    /// The object is on its way to that state and shows no fault: a rollout in progress,
    /// a pod that starts or ends.
    Progressing,
    /// The object does a part of its work, or it shows a sign of a fault.
    Warning,
    /// The object does not do its work.
    Failed,
}

impl ObjectHealth {
    /// True for an object that the "problems" filter of a table shows (plan section 11.2).
    pub fn is_problem(self) -> bool {
        self >= Self::Warning
    }
}

/// Gives the health value of an object. Each tier A kind has one rule, in the file of
/// the kind.
pub trait Assess {
    /// The health value of this object.
    fn health(&self) -> ObjectHealth;
}

/// One condition of the status of an object. Each kind has a type of its own for it.
pub(super) trait Condition {
    /// The name of the condition, for example `Ready`.
    fn name(&self) -> &str;

    /// `True`, `False` or `Unknown`.
    fn status(&self) -> &str;

    /// The reason of the last change. Empty with none.
    fn reason(&self) -> &str;
}

macro_rules! conditions {
    ($($condition:ty),*) => {
        $(impl Condition for $condition {
            fn name(&self) -> &str {
                &self.type_
            }

            fn status(&self) -> &str {
                &self.status
            }

            fn reason(&self) -> &str {
                self.reason.as_deref().unwrap_or_default()
            }
        })*
    };
}

conditions!(DeploymentCondition, ReplicaSetCondition, JobCondition, NamespaceCondition, NodeCondition, PodCondition);

/// The first condition with this name.
pub(super) fn condition<'a, C: Condition>(conditions: Option<&'a Vec<C>>, name: &str) -> Option<&'a C> {
    conditions?.iter().find(|condition| condition.name() == name)
}

/// True when the condition with this name has the status `True`.
pub(super) fn is_true<C: Condition>(conditions: Option<&Vec<C>>, name: &str) -> bool {
    condition(conditions, name).is_some_and(|condition| condition.status() == "True")
}

#[cfg(test)]
mod tests {
    use super::ObjectHealth::{Failed, Healthy, Progressing, Warning};

    #[test]
    fn the_worst_value_is_the_maximum_and_a_way_with_no_fault_is_no_problem() {
        assert_eq!([Healthy, Failed, Progressing, Warning].into_iter().max(), Some(Failed));
        assert_eq!(
            [Healthy, Progressing, Warning, Failed].map(super::ObjectHealth::is_problem),
            [false, false, true, true]
        );
    }
}
