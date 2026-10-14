//! The NetworkPolicy: its description. Its rules are a detail of its view.

use k8s_openapi::api::networking::v1::NetworkPolicy;

use super::describe::by_metadata;
use super::health::ObjectHealth;
use super::{Action, Column, Describe, Kind, Sidebar, Tier, Value, read};

impl Describe for NetworkPolicy {
    const ID: Kind = Kind::NetworkPolicy;
    const TITLE: &'static str = "NetworkPolicies";
    const SIDEBAR: Sidebar = Sidebar::Network;
    const TIER: Tier = Tier::B;
    const COLUMNS: &'static [Column<Self>] = &[
        Column::text("Pod selector", 180.0, |policy| {
            read::selector(policy.spec.as_ref().and_then(|spec| spec.pod_selector.as_ref()))
        }),
        Column::text("Policy types", 120.0, |policy| {
            let types = policy.spec.as_ref().and_then(|spec| spec.policy_types.as_ref());
            Value::list(types.into_iter().flatten().map(String::as_str))
        }),
    ];
    const ACTIONS: &'static [Action] = &[];

    fn health(&self) -> ObjectHealth {
        by_metadata(&self.metadata)
    }
}
