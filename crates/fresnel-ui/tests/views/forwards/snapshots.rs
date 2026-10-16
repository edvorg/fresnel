//! PNG files of the Forwards tab, of the dialog of a forward and of the Forward buttons of the
//! ports, for a look by a person.

use egui::{ViewportEvent, ViewportId};
use k8s_openapi::api::core::v1::{Pod, Service};

use super::*;
use crate::data::scene;
use crate::fake::{grow_dock, window_with};
use crate::snapshots::{SCALE, panel_of, save_clean, tall};
use crate::workloads::scene::allowed;

/// The window at the scale of a Retina display with the Forwards tab open, in a dock tall
/// enough for each line of the chosen forward.
fn tab(rows: Vec<ForwardRow>) -> Window {
    let mut window = window_with(with_forwards(rows), egui::Theme::Dark, SCALE);
    window.get_by_label("Forwards").click();
    window.run();
    grow_dock(&mut window, 250.0);
    window
}

#[test]
fn the_forwards_tab_with_a_forward_of_each_state() {
    let mut window = tab(forwards());
    save_clean(&mut window, "m5-forwards-tab");
    window.get_by_label("Forward of Deployment shop/api port 9090 in fra-prod").click();
    window.run();
    save_clean(&mut window, "m5-forwards-tab-reconnecting");
}

#[test]
fn the_dialog_of_a_forward_of_a_service_before_and_after_its_start() {
    let mut window = panel_of(vec![allowed::<Service>("ams-prod", &scene())], "Services", "web");
    let button = "Forward port 80 of the Service";
    window.get_by_label(button).scroll_to_me();
    window.run();
    save_clean(&mut window, "m5-service-ports");
    window.get_by_label(button).click();
    window.run();
    window.get_by_label("Fixed").click();
    window.run();
    crate::keys::typed(&mut window, "Fixed local port", "8080");
    save_clean(&mut window, "m5-forward-dialog");

    window.get_by_label("Start the forward").click();
    window.run();
    let rule = ForwardRule {
        cluster: ClusterId::new("ams-prod"),
        kind: Kind::Service,
        key: ObjectKey::new("scene", "web"),
        port: 80,
        local: LocalPort::Fixed(8080),
        bind: Bind::Localhost,
    };
    let active = ForwardView {
        addresses: addresses(&["127.0.0.1:8080", "[::1]:8080"]),
        target: pod("web-6888b778b6-75md7", 80, true),
        data: TargetData::Current,
        ..ForwardView::new(rule, ForwardState::Active, now())
    };
    window.state_mut().view.forwards = vec![ForwardRow { id: 1, saved: false, view: held(active) }].into();
    window.run();
    save_clean(&mut window, "m5-forward-started");
}

#[test]
fn the_forward_buttons_of_the_ports_of_a_pod() {
    let window = panel_of(vec![allowed::<Pod>("ams-prod", &pods())], "Pods", "web-6888b778b6-75md7");
    let mut window = tall(window, "CONTAINERS");
    save_clean(&mut window, "m5-pod-ports");
}

#[test]
fn the_question_at_a_close_while_forwards_run() {
    let mut window = tab(forwards());
    window.input_mut().viewports.entry(ViewportId::ROOT).or_default().events.push(ViewportEvent::Close);
    window.run();
    save_clean(&mut window, "m5-quit-question");
}
