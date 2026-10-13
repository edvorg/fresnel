//! The Deployment: its health value.

use k8s_openapi::api::apps::v1::{Deployment, DeploymentStatus};

use super::health::{Assess, Condition as _, ObjectHealth, condition, is_true};

/// The reason of the condition `Progressing` after a rollout that completed.
const ROLLED_OUT: &str = "NewReplicaSetAvailable";

impl Assess for Deployment {
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
