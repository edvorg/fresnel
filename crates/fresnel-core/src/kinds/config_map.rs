//! The ConfigMap: its description.

use k8s_openapi::api::core::v1::ConfigMap;

use super::describe::by_metadata;
use super::health::ObjectHealth;
use super::{Action, Column, Describe, Kind, Sidebar, Tier, Value};

impl Describe for ConfigMap {
    const ID: Kind = Kind::ConfigMap;
    const TITLE: &'static str = "ConfigMaps";
    const SIDEBAR: Sidebar = Sidebar::Config;
    const TIER: Tier = Tier::B;
    /// The keys of `data` and of `binaryData`: Lens shows their names, and Headlamp and
    /// kubectl their count.
    const COLUMNS: &'static [Column<Self>] = &[Column::text("Keys", 220.0, |map| {
        let binary = map.binary_data.iter().flatten().map(|(key, _)| key.as_str());
        Value::list(map.data.iter().flatten().map(|(key, _)| key.as_str()).chain(binary))
    })];
    const ACTIONS: &'static [Action] = &[Action::EditKeys, Action::AddKey, Action::RemoveKey];

    fn health(&self) -> ObjectHealth {
        by_metadata(&self.metadata)
    }
}
