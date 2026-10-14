//! The page of each kind on the fake core, with one kind of each group of the sidebar:
//! Nodes, Deployments, ConfigMaps, Services, ServiceAccounts and ControllerRevisions.
//!
//! - `scene`: the objects of these kinds, and a cluster with them.
//! - `groups`: the table and the detail panel of the kind of each group.
//! - `weakness`: the weaknesses 1, 3, 7 and 8 of plan section 6 for each of them.
//! - `sidebar`: the groups, the counts and the kinds that the core holds for the view.
//! - `columns`: the choice of the columns and the saved layout. `manifest`: the tab.
//! - `frame_time`: 10,000 Deployments. `snapshots`: PNG files of the pages.

mod columns;
mod frame_time;
mod groups;
mod manifest;
pub(crate) mod scene;
mod sidebar;
mod snapshots;
mod weakness;

use egui_kittest::kittest::Queryable as _;

use crate::fake::Window;

/// True when the detail panel shows this text: a node right of the left edge of its tabs.
pub(crate) fn in_panel(window: &Window, text: &str) -> bool {
    let Some(panel) = window.query_by_label("Overview").map(|tab| tab.rect().left() - 2.0) else {
        return false;
    };
    window.query_all_by_label(text).any(|node| node.rect().left() >= panel)
}
