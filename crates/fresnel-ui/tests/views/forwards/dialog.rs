//! The dialog of the Forward action: from the header of a panel, from the Forward button of
//! a port, from the menu of a row and from its key, also in a read-only cluster.

use egui::accesskit::Role;
use egui_kittest::kittest::NodeT as _;
use fresnel_core::config::forwards::save_edits;
use k8s_openapi::api::core::v1::{Pod, Service};

use super::*;
use crate::data::scene;
use crate::fake::{in_one_frame, press, shows, until};
use crate::keys::typed;
use crate::logs::copied;
use crate::settings::with_config;
use crate::workloads::scene::{allowed, opened};

/// The window with the panel of the Service `web` of the scene.
fn service_panel(config: &str) -> Window {
    let cluster = allowed::<Service>("ams-prod", &scene());
    let mut window = opened::<Service>(vec![cluster], "web");
    window.state_mut().view = with_config(window.state().view.clone(), config);
    window.run();
    window
}

fn starts(window: &Window) -> Vec<ForwardRule> {
    let starts = window.state().sent.iter().filter_map(|command| match command {
        Command::StartForward(rule) => Some(rule.clone()),
        _ => None,
    });
    starts.collect()
}

fn scene_rule(kind: Kind, name: &str, port: u16, local: LocalPort) -> ForwardRule {
    let key = ObjectKey::new("scene", name);
    ForwardRule { cluster: ClusterId::new("ams-prod"), kind, key, port, local, bind: Bind::Localhost }
}

/// The dialog starts a forward with a fixed local port, shows its state and its address from
/// the view of the core, copies the address and closes.
#[test]
fn the_dialog_of_a_service_starts_a_forward_and_shows_its_state_and_address() {
    let mut window = service_panel("version = 1\n");
    let button = "Forward port 80 of the Service";
    window.get_by_label(button).scroll_to_me();
    window.run();
    window.get_by_label(button).click();
    window.run();
    assert!(shows(&window, "Forward a port") && shows(&window, "Service scene/web in ams-prod"));
    // Port 80 is below 1024: the local port is automatic.
    let automatic = window.get_by_role_and_label(Role::RadioButton, "Automatic").accesskit_node().toggled();
    assert_eq!(automatic, Some(egui::accesskit::Toggled::True));
    window.get_by_label("Fixed").click();
    window.run();
    typed(&mut window, "Fixed local port", "8080");
    window.get_by_label("Save the forward in the settings: it starts with the app").click();
    window.run();
    window.get_by_label("Start the forward").click();
    window.run();
    let rule = scene_rule(Kind::Service, "web", 80, LocalPort::Fixed(8080));
    assert_eq!(starts(&window), std::slice::from_ref(&rule));
    assert!(window.state().sent.contains(&Command::EditSettings(save_edits(&rule))), "the save of the rule");
    assert!(shows(&window, "starting") && shows(&window, "The local port binds."));

    let active = ForwardView {
        addresses: vec!["127.0.0.1:8080".parse().expect("an address"), "[::1]:8080".parse().expect("an address")],
        ..ForwardView::new(rule.clone(), ForwardState::Active, now())
    };
    let row = ForwardRow { id: 1, saved: true, view: held(active) };
    window.state_mut().view.forwards = vec![row].into();
    window.run();
    assert!(shows(&window, "Address 127.0.0.1:8080"));
    window.get_by_label("Copy address").click();
    window.step();
    assert_eq!(copied(&window).as_deref(), Some("127.0.0.1:8080"));
    window.get_by_label("Close").click();
    window.run();
    assert!(!shows(&window, "Address 127.0.0.1:8080"), "the dialog closed");
    assert!(shows(&window, "1 forward: 1 active"), "the Forwards tab opened with the start");
}

/// The Forward button of the port of a container takes that port. A read-only cluster
/// forwards: a forward reads. A fixed local port needs its number before the start.
#[test]
fn the_forward_button_of_a_port_takes_its_port_also_in_a_read_only_cluster() {
    let cluster = allowed::<Pod>("ams-prod", &pods());
    let mut window = opened::<Pod>(vec![cluster], "web-6888b778b6-75md7");
    window.state_mut().view = with_config(window.state().view.clone(), "version = 1\n[safety]\nread_only = true\n");
    window.run();
    let button = "Forward port 80 of web";
    window.get_by_label(button).scroll_to_me();
    window.run();
    window.get_by_label(button).click();
    window.run();
    assert!(shows(&window, "Pod scene/web-6888b778b6-75md7 in ams-prod"));
    window.get_by_label("Fixed").click();
    window.run();
    assert!(shows(&window, "The local port is a number from 1 to 65535."));
    assert!(window.get_by_label("Start the forward").accesskit_node().is_disabled());
    typed(&mut window, "Fixed local port", "9000");
    window.get_by_label("Start the forward").click();
    window.run();
    let rule = scene_rule(Kind::Pod, "web-6888b778b6-75md7", 80, LocalPort::Fixed(9000));
    assert_eq!(starts(&window), [rule]);
}

/// The key of the action opens the dialog for the object of the panel, and the menu of a row
/// has the action too.
#[test]
fn the_key_and_the_menu_of_a_row_open_the_dialog() {
    let mut window = service_panel("version = 1\n");
    in_one_frame(&mut window, press(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::P));
    window.run();
    assert!(shows(&window, "Service scene/web in ams-prod"), "Cmd+Shift+P");
    in_one_frame(&mut window, press(egui::Modifiers::NONE, egui::Key::Escape));
    window.run();
    assert!(!shows(&window, "Port of the object"), "Escape closes it");

    let mut window = crate::fake::window(crate::fake::view(vec![allowed::<Service>("ams-prod", &scene())]));
    crate::fake::open_page(&mut window, "Services");
    until(&mut window, "the row of web", |window| window.query_by_role_and_label(Role::Label, "web").is_some());
    window.get_by_role_and_label(Role::Label, "web").click_secondary();
    window.run();
    until(&mut window, "the menu of the row", |window| window.query_by_label("Forward a port").is_some());
    window.get_by_label_contains("Forward a port").click();
    window.run();
    assert!(shows(&window, "Service scene/web in ams-prod"), "the menu of the row");
}
