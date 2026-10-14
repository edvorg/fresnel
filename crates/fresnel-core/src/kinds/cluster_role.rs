//! The ClusterRole: its description. Its rules are a detail of its view.

use k8s_openapi::api::rbac::v1::ClusterRole;

use super::describe::by_metadata;
use super::health::ObjectHealth;
use super::{Action, Column, Describe, Kind, Sidebar, Tier};

impl Describe for ClusterRole {
    const ID: Kind = Kind::ClusterRole;
    const TITLE: &'static str = "ClusterRoles";
    const SIDEBAR: Sidebar = Sidebar::AccessControl;
    const TIER: Tier = Tier::B;
    const COLUMNS: &'static [Column<Self>] = &[];
    const ACTIONS: &'static [Action] = &[];

    fn health(&self) -> ObjectHealth {
        by_metadata(&self.metadata)
    }
}
