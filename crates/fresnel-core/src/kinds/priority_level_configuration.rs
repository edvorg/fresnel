//! The PriorityLevelConfiguration: its description. It is one level of the flow control of
//! the API server, with its share of the requests and its queues.

use k8s_openapi::api::flowcontrol::v1::{PriorityLevelConfiguration, QueuingConfiguration};

use super::describe::by_metadata;
use super::health::ObjectHealth;
use super::{Action, Column, Describe, Kind, Sidebar, Tier, Value, read};

impl Describe for PriorityLevelConfiguration {
    const ID: Kind = Kind::PriorityLevelConfiguration;
    const TITLE: &'static str = "PriorityLevelConfigurations";
    const SIDEBAR: Sidebar = Sidebar::Other;
    const TIER: Tier = Tier::B;
    const COLUMNS: &'static [Column<Self>] = &[
        Column::text("Type", 80.0, |level| Value::text(level.spec.as_ref().map(|spec| spec.type_.as_str()))),
        Column::number("Nominal shares", 80.0, |level| {
            let limited = level.spec.as_ref().and_then(|spec| spec.limited.as_ref());
            read::count(limited.and_then(|limited| limited.nominal_concurrency_shares))
        }),
        Column::number("Queues", 64.0, |level| read::count(queuing(level).and_then(|queuing| queuing.queues))),
        Column::number("Hand size", 64.0, |level| read::count(queuing(level).and_then(|queuing| queuing.hand_size))),
        Column::number("Queue length limit", 90.0, |level| {
            read::count(queuing(level).and_then(|queuing| queuing.queue_length_limit))
        }),
    ];
    const ACTIONS: &'static [Action] = &[];

    fn health(&self) -> ObjectHealth {
        by_metadata(&self.metadata)
    }
}

/// The queues of a level that limits its requests. A level that rejects what it cannot
/// take at once has none.
fn queuing(level: &PriorityLevelConfiguration) -> Option<&QueuingConfiguration> {
    let limited = level.spec.as_ref()?.limited.as_ref()?;
    limited.limit_response.as_ref()?.queuing.as_ref()
}
