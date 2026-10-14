//! The Deployment: its description and its health value.

use k8s_openapi::api::apps::v1::{Deployment, DeploymentSpec, DeploymentStatus};

use super::health::{Condition as _, ObjectHealth, condition, is_true};
use super::{Action, Column, Describe, Kind, Sidebar, Tier, Value, read};

/// The reason of the condition `Progressing` after a rollout that completed.
const ROLLED_OUT: &str = "NewReplicaSetAvailable";

impl Describe for Deployment {
    const ID: Kind = Kind::Deployment;
    const TITLE: &'static str = "Deployments";
    const SIDEBAR: Sidebar = Sidebar::Workloads;
    const TIER: Tier = Tier::A;
    const COLUMNS: &'static [Column<Self>] = &[
        Column::number("Ready", 64.0, |deployment| {
            Value::Ratio(status(deployment, |status| status.ready_replicas), wanted(deployment))
        }),
        Column::number("Desired", 64.0, |deployment: &Self| Value::Number(wanted(deployment))).hidden(),
        Column::number("Updated", 64.0, |deployment| {
            Value::Number(status(deployment, |status| status.updated_replicas))
        }),
        Column::number("Available", 64.0, |deployment| {
            Value::Number(status(deployment, |status| status.available_replicas))
        }),
        Column::text("Conditions", 150.0, |deployment| {
            read::true_conditions(deployment.status.as_ref().and_then(|status| status.conditions.as_ref()))
        }),
        Column::text("Containers", 140.0, |deployment: &Self| read::containers(pod_spec(deployment))).hidden(),
        Column::text("Images", 220.0, |deployment| read::images(pod_spec(deployment))),
        Column::text("Selector", 160.0, |deployment: &Self| {
            read::selector(deployment.spec.as_ref().map(|spec| &spec.selector))
        })
        .hidden(),
    ];
    const ACTIONS: &'static [Action] = &[
        Action::Scale,
        Action::Restart,
        Action::RolloutStatus,
        Action::RevisionHistory,
        Action::Rollback,
        Action::Logs,
        Action::Forward,
    ];

    fn search<'a>(&'a self, texts: &mut Vec<&'a str>) {
        read::search_images(pod_spec(self), texts);
    }

    /// The steps follow `kubectl rollout status`:
    ///
    /// - Failed: the rollout passed its deadline, or the server refuses the pods.
    /// - Progressing: the controller did not see the spec yet, the rollout has pods to make
    ///   or to remove, or its new pods start.
    /// - After a rollout that completed, each pod that is not available is a fault of
    ///   the pods: a warning, or a failure when no pod is available.
    fn health(&self) -> ObjectHealth {
        let status = self.status.as_ref();
        let conditions = status.and_then(|status| status.conditions.as_ref());
        let progress = condition(conditions, "Progressing");
        if is_true(conditions, "ReplicaFailure") || progress.is_some_and(|progress| progress.status == "False") {
            return ObjectHealth::Failed;
        }
        let wanted = self.spec.as_ref().and_then(|spec| spec.replicas).unwrap_or(1);
        let count = |read: fn(&DeploymentStatus) -> Option<i32>| status.and_then(read).unwrap_or(0);
        let seen = status.and_then(|status| status.observed_generation) >= self.metadata.generation;
        let placed = (count(|status| status.replicas), count(|status| status.updated_replicas)) == (wanted, wanted);
        let available = count(|status| status.available_replicas);
        if !seen || !placed {
            ObjectHealth::Progressing
        } else if available >= wanted {
            ObjectHealth::Healthy
        } else if progress.is_none_or(|progress| progress.reason() != ROLLED_OUT) {
            ObjectHealth::Progressing
        } else if available == 0 {
            ObjectHealth::Failed
        } else {
            ObjectHealth::Warning
        }
    }
}

fn pod_spec(deployment: &Deployment) -> Option<&k8s_openapi::api::core::v1::PodSpec> {
    read::pod_spec(deployment.spec.as_ref().map(|spec: &DeploymentSpec| &spec.template))
}

/// The count of pods that the spec asks for. The server sets 1 when the manifest has none.
fn wanted(deployment: &Deployment) -> i64 {
    deployment.spec.as_ref().and_then(|spec| spec.replicas).unwrap_or(1).into()
}

/// A count of the status. The API reads an absent count as 0.
fn status(deployment: &Deployment, read: fn(&DeploymentStatus) -> Option<i32>) -> i64 {
    deployment.status.as_ref().and_then(read).unwrap_or(0).into()
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::fixture::{self, condition};
    use crate::kinds::ObjectHealth::{Failed, Healthy, Progressing, Warning};

    fn web(change: impl FnOnce(&mut Value)) -> ObjectHealth {
        fixture::changed::<Deployment>("web", change).health()
    }

    #[test]
    fn a_deployment_is_healthy_after_its_rollout_and_failed_after_the_deadline() {
        assert_eq!(fixture::one::<Deployment>("web").health(), Healthy);
        assert_eq!(fixture::one::<Deployment>("broken").health(), Failed, "the image does not exist");
        assert_eq!(
            web(|web| *condition(web, "ReplicaFailure") = json!({ "type": "ReplicaFailure", "status": "True" })),
            Failed
        );
        assert_eq!(web(|web| web["spec"]["replicas"] = 0.into()), Progressing, "two pods must go");
        let none = |web: &mut Value| {
            web["spec"]["replicas"] = 0.into();
            web["status"] = json!({ "observedGeneration": 2 });
        };
        assert_eq!(web(none), Healthy, "no pod by intent");
    }

    #[test]
    fn a_rollout_in_progress_is_no_fault_and_a_pod_that_is_down_after_it_is_one() {
        assert_eq!(web(|web| web["metadata"]["generation"] = 3.into()), Progressing, "the controller did not see it");
        assert_eq!(web(|web| web["status"]["updatedReplicas"] = 1.into()), Progressing);
        assert_eq!(web(|web| web["status"]["replicas"] = 3.into()), Progressing, "an old pod must go");
        let starts = |web: &mut Value| {
            web["status"]["availableReplicas"] = 1.into();
            condition(web, "Progressing")["reason"] = "ReplicaSetUpdated".into();
        };
        assert_eq!(web(starts), Progressing, "a new pod starts");

        assert_eq!(web(|web| web["status"]["availableReplicas"] = 1.into()), Warning);
        assert_eq!(web(|web| web["status"]["availableReplicas"] = 0.into()), Failed);
    }
}
