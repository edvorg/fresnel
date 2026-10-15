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

use fresnel_core::cluster::ClusterId;
use fresnel_core::kinds::Kind;
use fresnel_core::ops::Target;
use fresnel_core::store::ObjectKey;
use k8s_openapi::api::core::v1::Pod;

use crate::fake::{Window, grow_dock, window_with};
use crate::snapshots::write;

/// The size of the window of each picture, in points, and its scale. At the scale 2 of a
/// Retina display, three pictures are above the limit of 450 KB of `cargo xtask screenshots`.
const SIZE: egui::Vec2 = egui::vec2(1440.0, 900.0);
const SCALE: f32 = 1.5;
/// The margin that the harness puts around the app, in points. The window of the app has
/// none, so a picture leaves it out.
const MARGIN: f32 = 8.0;
/// A move of the wheel longer than a page or a panel: it scrolls to the end.
const TO_THE_END: f32 = -2000.0;
/// The dock grows this much over its first height, so it shows whole rows of the journal:
/// with the first height, half a row showed under the header of the journal.
const DOCK_GROWTH: f32 = 45.0;

/// The window of the app with the world of the showcase. The keys show with the names of a
/// Mac, as the README tells them.
fn window(theme: egui::Theme) -> Window {
    let mut window = window_with(clusters::world(), theme, SCALE);
    window.ctx.set_os(egui::os::OperatingSystem::Mac);
    window.set_size(SIZE + egui::Vec2::splat(2.0 * MARGIN));
    window.run();
    grow_dock(&mut window, DOCK_GROWTH);
    window
}

/// Saves the frame with no pointer arrow and with no margin of the harness. Each pixel of
/// the app is opaque; the margin was not, and a dialog dimmed it to a grey frame.
fn save(window: &mut Window, name: &str) {
    window.remove_cursor();
    window.run();
    let frame = window.render().expect("a GPU adapter draws the frame");
    let margin = (MARGIN * SCALE) as u32;
    let (width, height) = (frame.width() - 2 * margin, frame.height() - 2 * margin);
    let app = image::imageops::crop_imm(&frame, margin, margin, width, height).to_image();
    assert_eq!(app.dimensions(), ((SIZE.x * SCALE) as u32, (SIZE.y * SCALE) as u32));
    assert!(app.pixels().all(|pixel| pixel.0[3] == u8::MAX), "{name} has a pixel that is not opaque");
    write(&app, name);
}

/// The UID of an object of the world, as `objects::meta` gives it.
fn uid_of(cluster: &str, kind: Kind, namespace: &str, name: &str) -> String {
    objects::uid(&[cluster, kind.info().name, namespace, name])
}

/// An object of a cluster as the target of an operation.
fn target(cluster: &str, kind: Kind, namespace: &str, name: &str) -> Target {
    let uid = uid_of(cluster, kind, namespace, name);
    Target { cluster: ClusterId::new(cluster), kind, key: ObjectKey::new(namespace, name), uid: uid.into() }
}

/// The names of the pods of a cluster in the view of the window that `keep` keeps, sorted.
fn pods_of(window: &Window, cluster: &str, keep: impl Fn(&Pod) -> bool) -> Vec<String> {
    let clusters = &window.state().view.clusters;
    let row = clusters.iter().find(|row| row.id.as_str() == cluster).and_then(|row| row.data.as_ref());
    let data = row.unwrap_or_else(|| panic!("the data of {cluster}"));
    let mut names: Vec<String> =
        data.pods.items.iter().filter(|(_, pod)| keep(pod)).filter_map(|(_, pod)| pod.metadata.name.clone()).collect();
    names.sort();
    names
}
