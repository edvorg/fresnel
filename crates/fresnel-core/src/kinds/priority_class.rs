//! The PriorityClass: its description.

use k8s_openapi::api::scheduling::v1::PriorityClass;

use super::describe::by_metadata;
use super::health::ObjectHealth;
use super::{Action, Column, Describe, Kind, Sidebar, Tier, Value};

impl Describe for PriorityClass {
    const ID: Kind = Kind::PriorityClass;
    const TITLE: &'static str = "PriorityClasses";
    const SIDEBAR: Sidebar = Sidebar::Config;
    const TIER: Tier = Tier::B;
    const COLUMNS: &'static [Column<Self>] = &[
        Column::number("Value", 100.0, |class| Value::Number(class.value.into())),
        Column::text("Global default", 90.0, |class| Value::Flag(class.global_default == Some(true))),
        Column::text("Preemption policy", 140.0, |class| Value::text(class.preemption_policy.as_deref())),
    ];
    const ACTIONS: &'static [Action] = &[];

    fn health(&self) -> ObjectHealth {
        by_metadata(&self.metadata)
    }
}
