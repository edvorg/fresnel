//! The detail views of the kinds of the Network, Config, Access control and Cluster groups
//! on the fake core (milestone M7): the content of Headlamp and Lens for each kind, and the
//! links to the objects that an object names.
//!
//! - `network`, `config`, `access`, `cluster`: the views of the kinds of each group.
//! - `snapshots`: PNG files of a panel of each group.

mod access;
mod cluster;
mod config;
mod network;
mod snapshots;

use std::sync::Arc;

use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use fresnel_core::app::ClusterRow;
use fresnel_core::kinds::Describe;
use fresnel_core::store::StoreState;

use crate::data::{held, store};
use crate::fake::Window;

/// A cluster that holds the full objects of one more kind, as the core holds them for the
/// panel of a kind that reads them (`Command::ShowKinds`).
pub(crate) fn and<K: Describe>(row: ClusterRow, objects: &[Arc<K>]) -> ClusterRow {
    let mut data = row.data.clone().expect("data");
    data.objects = data.objects.with(held(store(StoreState::Live, objects)));
    ClusterRow { data: Some(data), ..row }
}

/// Clicks the link of the panel with this text: a cell of the table can have the same text.
pub(crate) fn follow(window: &mut Window, text: &str) {
    let link = window.query_all_by_label(text).find(|node| node.accesskit_node().role() == Role::Link);
    link.unwrap_or_else(|| panic!("the link {text}")).click_accesskit();
    window.run();
}

/// True when the frame shows a text that contains this one.
pub(crate) fn shows_part(window: &Window, text: &str) -> bool {
    window.query_all_by_label_contains(text).next().is_some()
}
