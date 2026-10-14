//! The DaemonSet: its description and its health value.

use k8s_openapi::api::apps::v1::{DaemonSet, DaemonSetStatus};
use k8s_openapi::api::core::v1::PodSpec;

use super::health::ObjectHealth;
use super::{Action, Column, Describe, Kind, Sidebar, Tier, Value, read};

impl Describe for DaemonSet {
    const ID: Kind = Kind::DaemonSet;
    const TITLE: &'static str = "DaemonSets";
    const SIDEBAR: Sidebar = Sidebar::Workloads;
    const TIER: Tier = Tier::A;
    const COLUMNS: &'static [Column<Self>] = &[
        Column::number("Desired", 64.0, |set| count(set, |status| Some(status.desired_number_scheduled))),
        Column::number("Current", 64.0, |set| count(set, |status| Some(status.current_number_scheduled))),
        Column::number("Ready", 64.0, |set| count(set, |status| Some(status.number_ready))),
        Column::number("Updated", 64.0, |set| count(set, |status| status.updated_number_scheduled)),
        Column::number("Available", 64.0, |set| count(set, |status| status.number_available)),
        Column::text("Node selector", 160.0, |set| {
            read::labels(pod_spec(set).and_then(|spec| spec.node_selector.as_ref()))
        }),
        Column::text("Containers", 140.0, |set: &Self| read::containers(pod_spec(set))).hidden(),
        Column::text("Images", 220.0, |set| read::images(pod_spec(set))),
        Column::text("Selector", 160.0, |set: &Self| read::selector(set.spec.as_ref().map(|spec| &spec.selector)))
            .hidden(),
    ];
    /// A DaemonSet has one pod on each node, so it has no scale.
    const ACTIONS: &'static [Action] = &[
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

    /// - Progressing: the controller did not see the spec yet, a node waits for its pod,
    ///   or an update did not reach each node.
    /// - Warning: each node has its pod, and not each pod is available, or a pod runs on
    ///   a node that must not have one.
    fn health(&self) -> ObjectHealth {
        let Some(status) = self.status.as_ref() else {
            return ObjectHealth::Progressing;
        };
        let wanted = status.desired_number_scheduled;
        let seen = status.observed_generation >= self.metadata.generation;
        let placed = status.current_number_scheduled.min(status.updated_number_scheduled.unwrap_or(0));
        if !seen || placed < wanted {
            ObjectHealth::Progressing
        } else if status.number_available.unwrap_or(0) >= wanted && status.number_misscheduled == 0 {
            ObjectHealth::Healthy
        } else {
            ObjectHealth::Warning
        }
    }
}

fn pod_spec(set: &DaemonSet) -> Option<&PodSpec> {
    read::pod_spec(set.spec.as_ref().map(|spec| &spec.template))
}

/// A count of the status. The API reads an absent count as 0.
fn count(set: &DaemonSet, read: fn(&DaemonSetStatus) -> Option<i32>) -> Value<'static> {
    Value::Number(set.status.as_ref().and_then(read).unwrap_or(0).into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture;
    use crate::kinds::ObjectHealth::{Healthy, Progressing, Warning};

    #[test]
    fn a_daemon_set_is_healthy_with_an_available_pod_on_each_node() {
        assert_eq!(fixture::one::<DaemonSet>("agent").health(), Healthy);
        let agent = |change: fn(&mut serde_json::Value)| fixture::changed::<DaemonSet>("agent", change).health();
        assert_eq!(agent(|set| set["status"]["numberAvailable"] = 0.into()), Warning);
        assert_eq!(agent(|set| set["status"]["numberMisscheduled"] = 1.into()), Warning);
        assert_eq!(agent(|set| set["status"]["desiredNumberScheduled"] = 2.into()), Progressing, "a new node");
        assert_eq!(agent(|set| set["status"]["updatedNumberScheduled"] = 0.into()), Progressing, "an update");
        assert_eq!(agent(|set| set["metadata"]["generation"] = 2.into()), Progressing);
        assert_eq!(DaemonSet::default().health(), Progressing, "no status yet");
    }
}
