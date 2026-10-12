//! Proves that this machine can run the UI tests: one frame with no window, drawn by the GPU code.
//!
//! When this test fails, each other UI test fails too. On Linux, the package
//! `mesa-vulkan-drivers` gives the software GPU that the test needs.

use std::collections::HashSet;

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;

#[test]
fn the_harness_lays_out_and_draws_one_frame() {
    let mut harness = Harness::new_ui(|ui| {
        ui.label("fresnel");
    });
    harness.run();
    harness.get_by_label("fresnel");

    let image = harness.render().expect("a GPU adapter draws the frame");
    let colors: HashSet<_> = image.pixels().collect();
    assert!(colors.len() > 1, "the frame shows the label on its background");
}
