//! PNG files of the Forwards tab and of the dialog of a forward, for a look by a person.

use k8s_openapi::api::core::v1::Service;

use super::*;
use crate::data::scene;
use crate::fake::{grow_dock, window_with};
use crate::snapshots::{SCALE, panel_of, save_clean};
use crate::workloads::scene::allowed;

#[test]
fn the_forwards_tab_with_a_forward_of_each_state() {
    let mut window = window_with(with_forwards(forwards()), egui::Theme::Dark, SCALE);
    window.get_by_label("Forwards").click();
    window.run();
    grow_dock(&mut window, 60.0);
    save_clean(&mut window, "m5-forwards-tab");
}

#[test]
fn the_dialog_of_a_forward_of_a_service() {
    let mut window = panel_of(vec![allowed::<Service>("ams-prod", &scene())], "Services", "web");
    let button = "Forward port 80 of the Service";
    window.get_by_label(button).scroll_to_me();
    window.run();
    window.get_by_label(button).click();
    window.run();
    window.get_by_label("Fixed").click();
    window.run();
    crate::keys::typed(&mut window, "Fixed local port", "8080");
    save_clean(&mut window, "m5-forward-dialog");
}
