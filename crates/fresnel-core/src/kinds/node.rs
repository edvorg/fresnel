//! The Node: its health value.

use k8s_openapi::api::core::v1::Node;

use super::health::{Assess, ObjectHealth, condition, is_true};

/// The conditions of a node that tell a shortage. `True` is the bad status of each one.
const SHORTAGES: [&str; 4] = ["MemoryPressure", "DiskPressure", "PIDPressure", "NetworkUnavailable"];

impl Assess for Node {
    /// - Failed: the node is not ready, or its kubelet does not report (`Unknown`).
    /// - Warning: the node is ready with a shortage, or it has no condition `Ready` yet.
    /// - A node that the user closed for new pods ("cordon") is healthy: that is a choice.
    fn health(&self) -> ObjectHealth {
        let conditions = self.status.as_ref().and_then(|status| status.conditions.as_ref());
        match condition(conditions, "Ready").map(|ready| ready.status.as_str()) {
            Some("True") if SHORTAGES.iter().any(|shortage| is_true(conditions, shortage)) => ObjectHealth::Warning,
            Some("True") => ObjectHealth::Healthy,
            Some(_) => ObjectHealth::Failed,
            None => ObjectHealth::Warning,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::{self, condition};
    use crate::kinds::ObjectHealth::{Failed, Healthy, Warning};

    #[test]
    fn a_node_that_is_not_ready_failed_and_a_shortage_is_a_warning() {
        let node = fixture::all::<Node>().remove(0);
        let name = node.metadata.name.clone().expect("a name");
        assert_eq!(node.health(), Healthy, "the node of k3s");
        let health = |change: fn(&mut serde_json::Value)| fixture::changed::<Node>(&name, change).health();
        assert_eq!(health(|node| condition(node, "Ready")["status"] = "False".into()), Failed);
        assert_eq!(health(|node| condition(node, "Ready")["status"] = "Unknown".into()), Failed, "no report");
        assert_eq!(health(|node| condition(node, "DiskPressure")["status"] = "True".into()), Warning);
        assert_eq!(health(|node| condition(node, "NetworkUnavailable")["status"] = "True".into()), Warning);
        assert_eq!(health(|node| node["spec"]["unschedulable"] = true.into()), Healthy, "a choice of the user");
        assert_eq!(Node::default().health(), Warning, "a new node with no condition");
    }
}
