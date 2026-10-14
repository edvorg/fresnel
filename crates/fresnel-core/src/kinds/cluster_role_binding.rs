//! The ClusterRoleBinding: its description.

use k8s_openapi::api::rbac::v1::ClusterRoleBinding;

use super::describe::by_metadata;
use super::health::ObjectHealth;
use super::{Action, Column, Describe, Kind, Sidebar, Tier, read};

impl Describe for ClusterRoleBinding {
    const ID: Kind = Kind::ClusterRoleBinding;
    const TITLE: &'static str = "ClusterRoleBindings";
    const SIDEBAR: Sidebar = Sidebar::AccessControl;
    const TIER: Tier = Tier::B;
    const COLUMNS: &'static [Column<Self>] = &[
        Column::text("Role", 180.0, |binding| read::role(&binding.role_ref)),
        Column::text("Users", 140.0, |binding| read::subjects(binding.subjects.as_ref(), "User")),
        Column::text("Groups", 140.0, |binding| read::subjects(binding.subjects.as_ref(), "Group")),
        Column::text("Service accounts", 180.0, |binding| read::subjects(binding.subjects.as_ref(), "ServiceAccount")),
    ];
    const ACTIONS: &'static [Action] = &[];

    fn health(&self) -> ObjectHealth {
        by_metadata(&self.metadata)
    }
}
