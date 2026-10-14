//! The Role: its description. Its rules are a detail of its view.

use k8s_openapi::api::rbac::v1::Role;

use super::describe::by_metadata;
use super::health::ObjectHealth;
use super::{Action, Column, Describe, Kind, Sidebar, Tier};

impl Describe for Role {
    const ID: Kind = Kind::Role;
    const TITLE: &'static str = "Roles";
    const SIDEBAR: Sidebar = Sidebar::AccessControl;
    const TIER: Tier = Tier::B;
    const COLUMNS: &'static [Column<Self>] = &[];
    const ACTIONS: &'static [Action] = &[];

    fn health(&self) -> ObjectHealth {
        by_metadata(&self.metadata)
    }
}
