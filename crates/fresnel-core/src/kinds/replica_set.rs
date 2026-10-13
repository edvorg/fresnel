//! The ReplicaSet: its health value.

use k8s_openapi::api::apps::v1::ReplicaSet;

use super::health::{Assess, ObjectHealth, is_true};

impl Assess for ReplicaSet {
    /// - Failed: the server refuses the pods.
    /// - Progressing: the controller did not see the spec yet, or it has pods to make or
    ///   to remove.
    /// - Warning: each pod exists, and not each one is ready. A ReplicaSet has no deadline,
    ///   so it does not tell a pod that starts from a pod that cannot start.
    fn health(&self) -> ObjectHealth {
        let status = self.status.as_ref();
        if is_true(status.and_then(|status| status.conditions.as_ref()), "ReplicaFailure") {
            return ObjectHealth::Failed;
        }
        let wanted = self.spec.as_ref().and_then(|spec| spec.replicas).unwrap_or(1);
        let seen = status.and_then(|status| status.observed_generation) >= self.metadata.generation;
        if !seen || status.map(|status| status.replicas) != Some(wanted) {
            ObjectHealth::Progressing
        } else if status.and_then(|status| status.ready_replicas).unwrap_or(0) >= wanted {
            ObjectHealth::Healthy
        } else {
            ObjectHealth::Warning
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::fixture::{self, condition};
    use crate::kinds::ObjectHealth::{Failed, Healthy, Progressing, Warning};

    #[test]
    fn a_replica_set_is_healthy_with_each_pod_ready_also_with_no_pod() {
        let all: Vec<ReplicaSet> = fixture::all()
            .into_iter()
            .filter(|set: &ReplicaSet| set.metadata.namespace.as_deref() == Some("scene"))
            .collect();
        let health = |replicas: i32, ready: i32| {
            let fits = |set: &&ReplicaSet| {
                let status = set.status.as_ref().expect("a status");
                (status.replicas, status.ready_replicas.unwrap_or(0)) == (replicas, ready)
            };
            all.iter().find(fits).expect("a ReplicaSet of the scene with these counts").health()
        };
        assert_eq!(health(2, 2), Healthy, "the current round of `web`");
        assert_eq!(health(0, 0), Healthy, "the round of `web` before");
        assert_eq!(health(1, 0), Warning, "the pod of `broken` cannot start");

        let web = |change: fn(&mut serde_json::Value)| fixture::changed::<ReplicaSet>("web-", change).health();
        assert_eq!(web(|set| set["spec"]["replicas"] = 5.into()), Progressing, "pods to make");
        assert_eq!(web(|set| set["metadata"]["generation"] = 9.into()), Progressing);
        assert_eq!(
            web(|set| *condition(set, "ReplicaFailure") = json!({ "type": "ReplicaFailure", "status": "True" })),
            Failed
        );
    }
}
