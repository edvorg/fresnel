//! The ReplicaSet: its description and its health value.

use k8s_openapi::api::apps::v1::ReplicaSet;
use k8s_openapi::api::core::v1::PodSpec;

use super::health::{ObjectHealth, is_true};
use super::{Action, Column, Describe, Kind, Sidebar, Tier, Value, read};

impl Describe for ReplicaSet {
    const ID: Kind = Kind::ReplicaSet;
    const TITLE: &'static str = "ReplicaSets";
    const SIDEBAR: Sidebar = Sidebar::Workloads;
    const TIER: Tier = Tier::A;
    const COLUMNS: &'static [Column<Self>] = &[
        Column::number("Desired", 64.0, |set| {
            Value::Number(set.spec.as_ref().and_then(|spec| spec.replicas).unwrap_or(1).into())
        }),
        Column::number("Current", 64.0, |set| read::count_or_zero(set.status.as_ref().map(|status| status.replicas))),
        Column::number("Ready", 64.0, |set| {
            read::count_or_zero(set.status.as_ref().and_then(|status| status.ready_replicas))
        }),
        Column::text("Containers", 140.0, |set: &Self| read::containers(pod_spec(set))).hidden(),
        Column::text("Images", 220.0, |set| read::images(pod_spec(set))),
        Column::text("Selector", 160.0, |set| read::selector(set.spec.as_ref().map(|spec| &spec.selector))),
        Column::number("Generation", 70.0, |set: &Self| Value::number(set.metadata.generation)).hidden(),
    ];
    const ACTIONS: &'static [Action] = &[Action::Scale, Action::Logs, Action::Forward];

    fn search<'a>(&'a self, texts: &mut Vec<&'a str>) {
        read::search_images(pod_spec(self), texts);
    }

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

fn pod_spec(set: &ReplicaSet) -> Option<&PodSpec> {
    read::pod_spec(set.spec.as_ref().and_then(|spec| spec.template.as_ref()))
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
