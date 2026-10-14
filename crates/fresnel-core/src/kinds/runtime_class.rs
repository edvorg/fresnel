//! The RuntimeClass: its description.

use k8s_openapi::api::node::v1::RuntimeClass;

use super::describe::by_metadata;
use super::health::ObjectHealth;
use super::{Action, Column, Describe, Kind, Sidebar, Tier, Value};

impl Describe for RuntimeClass {
    const ID: Kind = Kind::RuntimeClass;
    const TITLE: &'static str = "RuntimeClasses";
    const SIDEBAR: Sidebar = Sidebar::Config;
    const TIER: Tier = Tier::B;
    const COLUMNS: &'static [Column<Self>] =
        &[Column::text("Handler", 120.0, |class| Value::text(Some(&class.handler)))];
    const ACTIONS: &'static [Action] = &[];

    fn health(&self) -> ObjectHealth {
        by_metadata(&self.metadata)
    }
}
