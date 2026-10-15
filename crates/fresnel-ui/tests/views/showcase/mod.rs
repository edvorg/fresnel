//! The pictures of the README: the main views of the app on one made-up world of three
//! clusters, as on a Mac. `cargo xtask screenshots` runs these tests and compresses their PNG
//! files into `docs/screenshots`; in `cargo test`, they go to the test folder of the build.
//!
//! - `world`: the apps and the clusters. `stores`: the objects of each kind of a cluster.
//!   `objects`: their JSON. `usage`: the metrics. `clusters`: the view of the core.
//! - `scenes`: the pictures of the pages. `panels`: the pictures of a panel.

mod clusters;
mod objects;
mod panels;
mod scenes;
mod stores;
mod usage;
mod world;

use egui_kittest::kittest::Queryable as _;
use k8s_openapi::api::core::v1::Pod;

use crate::fake::{Window, window_with};

/// The size of the window of each picture, in points, and its scale. At the scale 2 of a
/// Retina display, three pictures are above the limit of 450 KB of `cargo xtask screenshots`.
const SIZE: egui::Vec2 = egui::vec2(1440.0, 900.0);
const SCALE: f32 = 1.5;

/// The window of the app with the world of the showcase. The keys show with the names of a
/// Mac, as the README tells them.
fn window(theme: egui::Theme) -> Window {
    let mut window = window_with(clusters::world(), theme, SCALE);
    window.ctx.set_os(egui::os::OperatingSystem::Mac);
    window.set_size(SIZE);
    window.run();
    window
}

/// A drag with the mouse from one place to an other, as on the edge of the dock or of the
/// panel: the pointer takes the edge in one frame and lets it go in a later one.
fn move_edge(window: &mut Window, from: egui::Pos2, to: egui::Pos2) {
    window.hover_at(from);
    window.run();
    window.drag_at(from);
    window.run();
    window.hover_at(to);
    window.run();
    window.drop_at(to);
    window.run();
}

/// The top edge of the dock: a drag of it changes the height of the dock.
fn dock_edge(window: &Window) -> egui::Pos2 {
    let tab = window.get_by_role_and_label(egui::accesskit::Role::Button, "Operations").rect();
    egui::pos2(tab.right() + 400.0, tab.top() - 4.0)
}

/// The names of the pods of prod-eu in the view of the window that `keep` keeps, sorted.
fn pods_of_prod_eu(window: &Window, keep: impl Fn(&Pod) -> bool) -> Vec<String> {
    let data = window.state().view.clusters.first().and_then(|row| row.data.as_ref()).expect("prod-eu");
    let mut names: Vec<String> =
        data.pods.items.iter().filter(|(_, pod)| keep(pod)).filter_map(|(_, pod)| pod.metadata.name.clone()).collect();
    names.sort();
    names
}
