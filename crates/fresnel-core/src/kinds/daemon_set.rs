//! The DaemonSet: its health value.

use k8s_openapi::api::apps::v1::DaemonSet;

use super::health::{Assess, ObjectHealth};

impl Assess for DaemonSet {
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
