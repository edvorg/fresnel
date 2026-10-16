//! The Edit action of the Manifest tab, of a row menu and of its key opens one editor for
//! each object, which reads its object once; a failed read reads again on a click, and the
//! close of a text with changes asks first.

use egui::accesskit::Role;
use egui::{Event, Key, Modifiers};
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use fresnel_core::app::EditState;
use k8s_openapi::api::apps::v1::Deployment;

use super::{WEB, click_row, edit, editors, give, panel, rows, text};
use crate::data::scene;
use crate::fake::{error, shows, until, view, window};
use crate::settings::with_config;
use crate::workloads::scene::{allowed, hover, open, target};

#[test]
fn the_edit_of_the_manifest_tab_opens_one_editor_with_the_text_of_its_object() {
    let mut window = panel();
    window.get_by_label("Manifest").click();
    window.run();
    window.get_by_label("Edit the manifest").click();
    window.run();
    let requests = editors(&window);
    let web = target::<Deployment>("ams-prod", &scene::<Deployment>(), "web");
    assert_eq!(requests.iter().map(|request| &request.target).collect::<Vec<_>>(), [&web]);
    assert!(shows(&window, "The manifest is read from ams-prod…"), "the dock shows the editor at once");

    give(&mut window, &requests[0], EditState::Done(text(WEB, None)));
    assert_eq!(
        rows(&window).get(..3),
        Some(["apiVersion: apps/v1", "kind: Deployment", "metadata:"].map(String::from).as_slice())
    );
    assert!(shows(&window, "Deployment scene/web in ams-prod") && shows(&window, "version 42 · 13 lines"));
    assert!(shows(&window, "Edit Deployment web"), "the title of the tab");

    // A second Edit of the object shows its tab, and asks for no other read.
    window.get_by_label("Edit the manifest").click();
    window.run();
    assert_eq!(editors(&window), requests);
    window.get_by_label("Close Edit Deployment web").click();
    window.run();
    assert_eq!(editors(&window), [], "the close drops the read");
    assert!(!shows(&window, "Edit Deployment web"));
}

#[test]
fn the_menu_of_a_row_and_its_key_open_the_editor_and_a_read_only_cluster_closes_it() {
    let deployments = scene::<Deployment>();
    let mut window = window(view(vec![allowed("ams-prod", &deployments)]));
    crate::fake::open_page(&mut window, "Deployments");
    until(&mut window, "the row", |window| window.query_all_by_role_and_label(Role::Label, "web").next().is_some());
    window.get_by_role_and_label(Role::Label, "web").click_secondary();
    window.run();
    window.get_by_label_contains("Edit manifest…").click();
    window.run();
    assert_eq!(editors(&window).len(), 1, "the menu of the row");
    window.get_by_label("Close Edit Deployment web").click();
    window.run();
    // The key acts on the object of the panel, which the menu of the row opened.
    window.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::Y);
    window.run();
    assert_eq!(editors(&window).len(), 1, "the key of the command table");

    let settings = "version = 1\n[clusters.ams-prod]\nread_only = true\n";
    let mut window = window_read_only(settings);
    window.get_by_label("Manifest").click();
    window.run();
    assert!(window.get_by_label("Edit the manifest").accesskit_node().is_disabled());
    hover(&mut window, Role::Button, "Edit the manifest");
    assert!(shows(&window, "ams-prod is read-only in the settings: the app makes no change in it."));
}

fn window_read_only(settings: &str) -> crate::fake::Window {
    let deployments = scene::<Deployment>();
    let mut window = window(with_config(view(vec![allowed("ams-prod", &deployments)]), settings));
    open(&mut window, "Deployments", "web");
    window
}

#[test]
fn a_failed_read_tells_why_and_reads_again_on_a_click() {
    let mut window = panel();
    window.get_by_label("Manifest").click();
    window.run();
    window.get_by_label("Edit the manifest").click();
    window.run();
    let first = editors(&window).pop().expect("the request of the editor");
    give(&mut window, &first, EditState::Failed(error("the request got no answer in 10s")));
    assert!(shows(&window, "The read failed: the request got no answer in 10s"));
    window.get_by_label("Read again").click();
    window.run();
    let again = editors(&window);
    assert_eq!(again.len(), 1);
    assert!(again[0].number != first.number && again[0].target == first.target, "a new request of the object");
}

#[test]
fn the_close_of_a_text_with_changes_asks_first() {
    let mut window = panel();
    edit(&mut window, text(WEB, None));
    click_row(&mut window, "  replicas: 2");
    window.key_press(Key::Backspace);
    window.event(Event::Text("3".to_owned()));
    window.run();
    assert!(shows(&window, "Edit Deployment web •") && rows(&window).contains(&"  replicas: 3".to_owned()));
    window.get_by_label("Close Edit Deployment web •").click();
    window.run();
    assert!(shows(&window, "Close the editor of Deployment scene/web in ams-prod?"));
    window.get_by_label("Cancel").click();
    window.run();
    assert_eq!(editors(&window).len(), 1, "the text stays");
    window.get_by_label("Close Edit Deployment web •").click();
    window.run();
    window.get_by_label("Close and drop the changes").click();
    window.run();
    assert_eq!(editors(&window), []);
}

/// A key that is text goes into the text of the editor, not to a command: `?` shows the
/// keys elsewhere.
#[test]
fn a_question_mark_in_the_editor_is_text() {
    let mut window = panel();
    edit(&mut window, text(WEB, None));
    click_row(&mut window, "kind: Deployment");
    window.key_press(Key::Questionmark);
    window.event(Event::Text("?".to_owned()));
    window.run();
    assert!(rows(&window).contains(&"kind: Deployment?".to_owned()));
    assert!(!shows(&window, "The object of the panel"), "no overlay of the keys");
}
