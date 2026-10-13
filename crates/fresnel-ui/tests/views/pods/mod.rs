//! The Pods view on the fake core.
//!
//! - `states`: each state of a store. `table`: sort, filters, counts and the selection.
//! - `weakness`: the weaknesses of Headlamp of plan section 6 that apply to this view.
//! - `detail`: the detail panel. `snapshots`: PNG files of the view.
//! - `bad_data`: objects that a healthy API server does not give.
//! - `rest`: the view while the window shows some other page.

mod bad_data;
mod detail;
mod rest;
pub(crate) mod scenes;
mod snapshots;
mod states;
mod table;
mod weakness;

use egui_kittest::kittest::{NodeT as _, Queryable as _};

use crate::fake::Window;

/// The texts of one column of the rows on the screen, from the top, with the freshness
/// marks of that column. The cells of a column are the labels below its title and above
/// the dock. The middle of a cell decides, so a row that shows in part counts in each column.
fn cells(window: &Window, title: &str) -> Vec<String> {
    let header = window.get_by_role_and_label(egui::accesskit::Role::Button, title).rect();
    let dock = window.get_by_role_and_label(egui::accesskit::Role::Button, "Journal").rect();
    let in_column = |rect: egui::Rect| {
        let middle = rect.center();
        header.x_range().contains(middle.x) && middle.y > header.bottom() && middle.y < dock.top()
    };
    let mut cells: Vec<(f32, String)> = window
        .query_all_by_role(egui::accesskit::Role::Label)
        .filter(|node| in_column(node.rect()))
        .map(|node| (node.rect().center().y, node.accesskit_node().value().unwrap_or_default()))
        .collect();
    cells.sort_by(|a, b| a.0.total_cmp(&b.0));
    cells.into_iter().map(|(_, text)| text).collect()
}

/// The texts of one column of the rows on the screen, from the top.
pub(crate) fn column(window: &Window, title: &str) -> Vec<String> {
    cells(window, title).into_iter().filter(|text| !text.starts_with("old")).collect()
}

/// The count of rows on the screen with this freshness mark.
pub(crate) fn marks(window: &Window, mark: &str) -> usize {
    cells(window, "Name").iter().filter(|text| *text == mark).count()
}
