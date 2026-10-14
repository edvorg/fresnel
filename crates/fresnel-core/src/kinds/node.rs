//! The Node: its description and its health value. CPU and memory come from the metrics
//! of M6.

use std::borrow::Cow;
use std::collections::BTreeSet;

use k8s_openapi::api::core::v1::{Node, NodeSystemInfo};

use super::health::{ObjectHealth, condition, is_true};
use super::{Action, Column, Describe, Kind, Sidebar, Tier, Value, read};

/// The conditions of a node that tell a shortage. `True` is the bad status of each one.
const SHORTAGES: [&str; 4] = ["MemoryPressure", "DiskPressure", "PIDPressure", "NetworkUnavailable"];
/// The labels that name the role of a node: `node-role.kubernetes.io/<role>`, and the
/// older `kubernetes.io/role: <role>`.
const ROLE_PREFIX: &str = "node-role.kubernetes.io/";
const ROLE_LABEL: &str = "kubernetes.io/role";
/// The labels that name the node pool in the managed clusters of DigitalOcean, Google,
/// Amazon, Azure and of Karpenter.
const POOL_LABELS: [&str; 5] = [
    "doks.digitalocean.com/node-pool",
    "cloud.google.com/gke-nodepool",
    "eks.amazonaws.com/nodegroup",
    "kubernetes.azure.com/agentpool",
    "karpenter.sh/nodepool",
];
const INSTANCE_TYPE: &str = "node.kubernetes.io/instance-type";

impl Describe for Node {
    const ID: Kind = Kind::Node;
    const TITLE: &'static str = "Nodes";
    const SIDEBAR: Sidebar = Sidebar::Cluster;
    const TIER: Tier = Tier::A;
    const COLUMNS: &'static [Column<Self>] = &[
        Column::text("Status", 150.0, status),
        Column::text("Roles", 140.0, roles),
        Column::number("Taints", 56.0, |node| {
            let taints = node.spec.as_ref().and_then(|spec| spec.taints.as_ref());
            Value::Number(read::number_of(taints.map_or(0, Vec::len)))
        }),
        Column::text("Version", 120.0, |node| Value::text(info(node).map(|info| info.kubelet_version.as_str()))),
        Column::text("Internal IP", 110.0, |node| Value::text(address(node, "InternalIP"))),
        Column::text("External IP", 110.0, |node| Value::text(address(node, "ExternalIP"))),
        Column::text("Node pool", 120.0, |node: &Self| {
            let labels = node.metadata.labels.as_ref();
            Value::text(POOL_LABELS.iter().find_map(|label| labels?.get(*label)).map(String::as_str))
        })
        .hidden(),
        Column::text("Instance type", 120.0, |node: &Self| {
            Value::text(node.metadata.labels.as_ref().and_then(|labels| labels.get(INSTANCE_TYPE)).map(String::as_str))
        })
        .hidden(),
        Column::text("OS image", 160.0, |node: &Self| Value::text(info(node).map(|info| info.os_image.as_str())))
            .hidden(),
        Column::text("Kernel", 140.0, |node: &Self| Value::text(info(node).map(|info| info.kernel_version.as_str())))
            .hidden(),
        Column::text("Container runtime", 160.0, |node: &Self| {
            Value::text(info(node).map(|info| info.container_runtime_version.as_str()))
        })
        .hidden(),
        Column::text("Conditions", 150.0, |node: &Self| {
            read::true_conditions(node.status.as_ref().and_then(|status| status.conditions.as_ref()))
        })
        .hidden(),
    ];
    const ACTIONS: &'static [Action] = &[Action::Cordon, Action::Uncordon, Action::Drain, Action::NodeShell];

    /// The addresses of the node: its IP addresses and its host name.
    fn search<'a>(&'a self, texts: &mut Vec<&'a str>) {
        let addresses = self.status.as_ref().and_then(|status| status.addresses.as_ref());
        texts.extend(addresses.into_iter().flatten().map(|address| address.address.as_str()));
    }

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

fn info(node: &Node) -> Option<&NodeSystemInfo> {
    node.status.as_ref()?.node_info.as_ref()
}

/// The first address of the node of this type.
fn address<'a>(node: &'a Node, kind: &str) -> Option<&'a str> {
    let addresses = node.status.as_ref()?.addresses.as_ref()?;
    addresses.iter().find(|address| address.type_ == kind).map(|address| address.address.as_str())
}

/// The status of kubectl: `Ready`, `NotReady` or `Unknown`, and `SchedulingDisabled` for
/// a node that the user closed for new pods.
fn status(node: &Node) -> Value<'static> {
    let conditions = node.status.as_ref().and_then(|status| status.conditions.as_ref());
    let ready = match condition(conditions, "Ready").map(|ready| ready.status.as_str()) {
        Some("True") => "Ready",
        Some(_) => "NotReady",
        None => "Unknown",
    };
    if node.spec.as_ref().and_then(|spec| spec.unschedulable) == Some(true) {
        Value::Text(Cow::Owned(format!("{ready},SchedulingDisabled")))
    } else {
        Value::Text(Cow::Borrowed(ready))
    }
}

/// The roles of the labels of the node, in the sequence of the alphabet.
fn roles(node: &Node) -> Value<'_> {
    let labels = node.metadata.labels.iter().flatten();
    let roles: BTreeSet<&str> = labels
        .filter_map(|(key, value)| match key.strip_prefix(ROLE_PREFIX) {
            Some(role) => Some(role),
            None => (key == ROLE_LABEL).then_some(value.as_str()),
        })
        .filter(|role| !role.is_empty())
        .collect();
    Value::list(roles)
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
