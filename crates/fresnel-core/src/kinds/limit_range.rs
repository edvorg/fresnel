//! The LimitRange: its description. Its limits are a detail of its view.

use k8s_openapi::api::core::v1::LimitRange;

use super::describe::by_metadata;
use super::health::ObjectHealth;
use super::{Action, Column, Describe, Kind, Sidebar, Tier};

impl Describe for LimitRange {
    const ID: Kind = Kind::LimitRange;
    const TITLE: &'static str = "LimitRanges";
    const SIDEBAR: Sidebar = Sidebar::Config;
    const TIER: Tier = Tier::B;
    const COLUMNS: &'static [Column<Self>] = &[];
    const ACTIONS: &'static [Action] = &[];

    fn health(&self) -> ObjectHealth {
        by_metadata(&self.metadata)
    }
}
