//! The manifest editor of the dock (plan section 12.9, milestone M11): its open from the
//! Manifest tab, a menu and a key, its read, its text with colors, line numbers, marks and a
//! search, its keys and its undo.
//!
//! - `open`: the Edit action and the tabs. `text`: the check, the marks, the search, the
//!   buttons. `secret`: the values of a Secret. `limit`: a text past 8,000 lines.
//! - `frame_time`: a key press in 8,000 lines in a release build. `snapshots`: PNG files.

mod frame_time;
mod limit;
mod open;
mod secret;
mod snapshots;
mod text;

use std::sync::Arc;

use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use fresnel_core::app::{Command, EditRead, EditRequest, EditState};
use fresnel_core::manifest::{Decoded, EditText};
use k8s_openapi::api::apps::v1::Deployment;

use crate::data::scene;
use crate::fake::{Window, until, view, window};
use crate::workloads::scene::{allowed, open};

/// The text of the Deployment `web` of the scene, as the core writes it.
pub(crate) const WEB: &str = "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: web\n  namespace: scene\n  \
                              resourceVersion: \"42\"\nspec:\n  replicas: 2\n  template:\n    spec:\n      containers:\n      \
                              - image: nginx:1.27\n        name: web\n";

/// The text of an editor as the core gives it.
pub(crate) fn text(yaml: &str, decoded: Option<Decoded>) -> Arc<EditText> {
    let version = Some("42".to_owned());
    Arc::new(EditText { text: yaml.into(), version, lines: yaml.lines().count(), decoded })
}

/// The requests of the last `Command::ShowEditors` of the views.
pub(crate) fn editors(window: &Window) -> Vec<EditRequest> {
    let mut sent = window.state().sent.iter().rev();
    let requests = sent.find_map(|command| match command {
        Command::ShowEditors(requests) => Some(requests.clone()),
        _ => None,
    });
    requests.unwrap_or_default()
}

/// Gives the fake core the state of the read of an editor.
pub(crate) fn give(window: &mut Window, request: &EditRequest, state: EditState) {
    window.state_mut().view.edits = vec![Arc::new(EditRead { request: request.clone(), state })];
    window.run();
}

/// The window with the panel of the Deployment `web` of ams-prod.
pub(crate) fn panel() -> Window {
    let deployments = scene::<Deployment>();
    let mut window = window(view(vec![allowed("ams-prod", &deployments)]));
    open(&mut window, "Deployments", "web");
    window
}

/// Opens the editor of the panel with the button of its Manifest tab, and gives it this
/// text. Gives the request of the editor.
pub(crate) fn edit(window: &mut Window, yaml: Arc<EditText>) -> EditRequest {
    window.get_by_label("Manifest").click();
    window.run();
    window.get_by_label("Edit the manifest").click();
    window.run();
    let request = editors(window).pop().expect("the request of the editor");
    give(window, &request, EditState::Done(yaml));
    request
}

/// The text of each row of the editor on the screen, from the top.
pub(crate) fn rows(window: &Window) -> Vec<String> {
    let mut rows: Vec<(f32, String)> = window
        .query_all_by_role(Role::ListItem)
        .map(|row| (row.rect().top(), row.accesskit_node().label().unwrap_or_default()))
        .collect();
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    rows.into_iter().map(|(_, text)| text).collect()
}

/// Draws frames until the check of the text shows this state in the bar.
pub(crate) fn checked(window: &mut Window, state: &str) {
    until(window, &format!("the check: {state}"), |window| window.query_by_label(state).is_some());
}

/// Gives the text of the editor the focus, with its cursor at the end of the row with
/// this text.
pub(crate) fn click_row(window: &mut Window, row: &str) {
    window.get_by_role_and_label(Role::ListItem, row).click();
    window.run();
}
