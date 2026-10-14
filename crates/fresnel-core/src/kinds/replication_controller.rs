//! The ReplicationController: its description and its health value. It is the older form
//! of a ReplicaSet, with the same rule.

use k8s_openapi::api::core::v1::{PodSpec, ReplicationController};

use super::health::{ObjectHealth, is_true};
use super::{Action, Column, Describe, Kind, Sidebar, Tier, Value, read};

impl Describe for ReplicationController {
    const ID: Kind = Kind::ReplicationController;
    const TITLE: &'static str = "ReplicationControllers";
    const SIDEBAR: Sidebar = Sidebar::Workloads;
    const TIER: Tier = Tier::B;
    const COLUMNS: &'static [Column<Self>] = &[
        Column::number("Desired", 64.0, |controller| Value::Number(wanted(controller).into())),
        Column::number("Current", 64.0, |controller| {
            read::count_or_zero(controller.status.as_ref().map(|status| status.replicas))
        }),
        Column::number("Ready", 64.0, |controller| {
            read::count_or_zero(controller.status.as_ref().and_then(|status| status.ready_replicas))
        }),
        Column::text("Containers", 140.0, |controller: &Self| read::containers(pod_spec(controller))).hidden(),
        Column::text("Images", 220.0, |controller| read::images(pod_spec(controller))),
        Column::text("Selector", 160.0, |controller| {
            read::labels(controller.spec.as_ref().and_then(|spec| spec.selector.as_ref()))
        }),
    ];
    const ACTIONS: &'static [Action] = &[Action::Scale, Action::Logs, Action::Forward];

    /// The rule of a ReplicaSet: failed when the server refuses the pods, on its way while
    /// it has pods to make or to remove, and a warning while a pod is not ready.
    fn health(&self) -> ObjectHealth {
        let status = self.status.as_ref();
        if is_true(status.and_then(|status| status.conditions.as_ref()), "ReplicaFailure") {
            return ObjectHealth::Failed;
        }
        let seen = status.and_then(|status| status.observed_generation) >= self.metadata.generation;
        if !seen || status.map(|status| status.replicas) != Some(wanted(self)) {
            ObjectHealth::Progressing
        } else if status.and_then(|status| status.ready_replicas).unwrap_or(0) >= wanted(self) {
            ObjectHealth::Healthy
        } else {
            ObjectHealth::Warning
        }
    }

    fn search<'a>(&'a self, texts: &mut Vec<&'a str>) {
        read::search_images(pod_spec(self), texts);
    }
}

fn pod_spec(controller: &ReplicationController) -> Option<&PodSpec> {
    read::pod_spec(controller.spec.as_ref().and_then(|spec| spec.template.as_ref()))
}

/// The count of pods that the spec asks for. The server sets 1 when the manifest has none.
fn wanted(controller: &ReplicationController) -> i32 {
    controller.spec.as_ref().and_then(|spec| spec.replicas).unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::fixture::{self, condition};
    use crate::kinds::ObjectHealth::{Failed, Healthy, Progressing, Warning};

    fn legacy(change: impl FnOnce(&mut Value)) -> ObjectHealth {
        fixture::of_kinds::<ReplicationController>("legacy", change).health()
    }

    #[test]
    fn a_replication_controller_has_the_rule_of_a_replica_set() {
        assert_eq!(legacy(|_| {}), Healthy);
        assert_eq!(legacy(|rc| rc["status"]["readyReplicas"] = 0.into()), Warning, "its pod is not ready");
        assert_eq!(legacy(|rc| rc["spec"]["replicas"] = 3.into()), Progressing, "pods to make");
        assert_eq!(legacy(|rc| rc["metadata"]["generation"] = 9.into()), Progressing, "the controller did not see it");
        let refused =
            |rc: &mut Value| *condition(rc, "ReplicaFailure") = json!({ "type": "ReplicaFailure", "status": "True" });
        assert_eq!(legacy(refused), Failed);
    }
}
