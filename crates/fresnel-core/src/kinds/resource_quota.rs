//! The ResourceQuota: its description.

use k8s_openapi::api::core::v1::ResourceQuota;

use super::describe::by_metadata;
use super::health::ObjectHealth;
use super::{Action, Column, Describe, Kind, Sidebar, Tier, Value};

/// The start of the name of a resource that limits the pods. kubectl shows these in the
/// column of the limits, and each other one in the column of the requests.
const LIMITS: &str = "limits.";

impl Describe for ResourceQuota {
    const ID: Kind = Kind::ResourceQuota;
    const TITLE: &'static str = "ResourceQuotas";
    const SIDEBAR: Sidebar = Sidebar::Config;
    const TIER: Tier = Tier::B;
    const COLUMNS: &'static [Column<Self>] = &[
        Column::text("Request", 260.0, |quota| usage(quota, false)),
        Column::text("Limit", 200.0, |quota| usage(quota, true)),
    ];
    const ACTIONS: &'static [Action] = &[];

    fn health(&self) -> ObjectHealth {
        by_metadata(&self.metadata)
    }
}

/// The use and the limit of each resource, as kubectl shows them: `pods: 1/10`. The
/// values come from the status, which the server keeps in the canonical form.
fn usage(quota: &ResourceQuota, limits: bool) -> Value<'static> {
    let status = quota.status.as_ref();
    let used = status.and_then(|status| status.used.as_ref());
    let hard = status.and_then(|status| status.hard.as_ref()).into_iter().flatten();
    let entries = hard.filter(|(resource, _)| resource.starts_with(LIMITS) == limits).map(|(resource, limit)| {
        let used = used.and_then(|used| used.get(resource)).map_or("0", |used| used.0.as_str());
        format!("{resource}: {used}/{}", limit.0)
    });
    Value::texts(entries)
}
