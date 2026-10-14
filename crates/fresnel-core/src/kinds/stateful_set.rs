//! The StatefulSet: its description and its health value.

use k8s_openapi::api::apps::v1::StatefulSet;
use k8s_openapi::api::core::v1::PodSpec;

use super::health::ObjectHealth;
use super::{Action, Column, Describe, Kind, Sidebar, Tier, Value, read};

impl Describe for StatefulSet {
    const ID: Kind = Kind::StatefulSet;
    const TITLE: &'static str = "StatefulSets";
    const SIDEBAR: Sidebar = Sidebar::Workloads;
    const TIER: Tier = Tier::A;
    const COLUMNS: &'static [Column<Self>] = &[
        Column::number("Ready", 64.0, |set| {
            let ready = set.status.as_ref().and_then(|status| status.ready_replicas).unwrap_or(0);
            Value::Ratio(ready.into(), wanted(set).into())
        }),
        Column::number("Desired", 64.0, |set: &Self| Value::Number(wanted(set).into())).hidden(),
        Column::text("Containers", 140.0, |set: &Self| read::containers(pod_spec(set))).hidden(),
        Column::text("Images", 220.0, |set| read::images(pod_spec(set))),
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

    /// - Progressing: the controller did not see the spec yet, it has pods to make or to
    ///   remove, or an update moves the pods to a new revision.
    /// - Warning: each pod exists at the current revision, and not each one is ready.
    fn health(&self) -> ObjectHealth {
        let Some(status) = self.status.as_ref() else {
            return ObjectHealth::Progressing;
        };
        let wanted = self.spec.as_ref().and_then(|spec| spec.replicas).unwrap_or(1);
        let seen = status.observed_generation >= self.metadata.generation;
        let update = status.update_revision.is_some() && status.update_revision != status.current_revision;
        if !seen || status.replicas != wanted || update {
            ObjectHealth::Progressing
        } else if status.ready_replicas.unwrap_or(0) >= wanted {
            ObjectHealth::Healthy
        } else {
            ObjectHealth::Warning
        }
    }
}

fn pod_spec(set: &StatefulSet) -> Option<&PodSpec> {
    read::pod_spec(set.spec.as_ref().map(|spec| &spec.template))
}

/// The count of pods that the spec asks for. The server sets 1 when the manifest has none.
fn wanted(set: &StatefulSet) -> i32 {
    set.spec.as_ref().and_then(|spec| spec.replicas).unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture;
    use crate::kinds::ObjectHealth::{Healthy, Progressing, Warning};

    #[test]
    fn a_stateful_set_is_on_its_way_in_an_update_and_a_pod_that_is_not_ready_is_a_warning() {
        assert_eq!(fixture::one::<StatefulSet>("db").health(), Healthy);
        let db = |change: fn(&mut serde_json::Value)| fixture::changed::<StatefulSet>("db", change).health();
        assert_eq!(db(|set| set["status"]["readyReplicas"] = 1.into()), Warning);
        assert_eq!(db(|set| set["status"]["updateRevision"] = "db-new".into()), Progressing);
        assert_eq!(db(|set| set["status"]["replicas"] = 1.into()), Progressing, "a pod to make");
        assert_eq!(db(|set| set["metadata"]["generation"] = 2.into()), Progressing);
        assert_eq!(db(|set| set["spec"]["replicas"] = 0.into()), Progressing, "two pods must go");
        assert_eq!(StatefulSet::default().health(), Progressing, "no status yet");
    }
}
