//! The ControllerRevision: its description. A StatefulSet and a DaemonSet keep each
//! revision of their pod template in one, for the revision history and the rollback.

use k8s_openapi::api::apps::v1::ControllerRevision;

use super::describe::by_metadata;
use super::health::ObjectHealth;
use super::{Action, Column, Describe, Kind, Sidebar, Tier, Value, read};

impl Describe for ControllerRevision {
    const ID: Kind = Kind::ControllerRevision;
    const TITLE: &'static str = "ControllerRevisions";
    const SIDEBAR: Sidebar = Sidebar::Other;
    const TIER: Tier = Tier::B;
    const COLUMNS: &'static [Column<Self>] = &[
        Column::text("Controller", 200.0, |revision| read::controller(&revision.metadata)),
        Column::number("Revision", 70.0, |revision| Value::Number(revision.revision)),
    ];
    const ACTIONS: &'static [Action] = &[];

    fn health(&self) -> ObjectHealth {
        by_metadata(&self.metadata)
    }
}
