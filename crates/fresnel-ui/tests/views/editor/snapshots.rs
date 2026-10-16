//! PNG files of the editor: the colors and the line numbers in each theme, and in the dark
//! theme an error and a hint with their marks, the hits of a search, a Secret, and a text
//! past the line limit.

use egui::accesskit::Role;
use egui::{Event, Key, Modifiers};
use egui_kittest::kittest::Queryable as _;
use fresnel_core::app::EditState;
use fresnel_core::manifest::Decoded;
use k8s_openapi::api::apps::v1::Deployment;

use super::{checked, click_row, editors, give, text};
use crate::data::scene;
use crate::fake::{Window, until, view, window_with};
use crate::keys::scene::{secret, with_secrets};
use crate::snapshots::{SCALE, save_clean};
use crate::workloads::scene::allowed;

/// A Deployment as the core writes it, with a comment, a block scalar and flow values.
const MANIFEST: &str = "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  annotations:\n    \
    deployment.kubernetes.io/revision: \"3\"\n  labels:\n    app: web\n    tier: frontend\n  name: web\n  \
    namespace: scene\n  resourceVersion: \"8812\"\nspec:\n  replicas: 3\n  selector:\n    matchLabels:\n      \
    app: web\n  template:\n    metadata:\n      labels:\n        app: web\n    spec:\n      containers:\n      \
    - args: [\"--port\", \"8080\"]\n        env:\n        - name: MODE\n          value: production\n        \
    - name: CONFIG\n          value: |\n            listen 8080;\n            root /srv;\n        \
    image: nginx:1.27\n        name: web\n        ports:\n        - containerPort: 8080\n          \
    protocol: TCP\n        readinessProbe:\n          httpGet:\n            path: /healthz\n            \
    port: 8080\n      restartPolicy: Always\n";

/// A Secret as the core writes it for the editor: two text values and a binary one.
fn secret_text() -> String {
    let binary = "MIIKcAIBAzCCCiYGCSqG".repeat(5);
    format!(
        "apiVersion: v1\ndata:\n  keystore.p12: !!binary {binary}\n  password: hunter2\n  tls.crt: |\n    -----BEGIN \
         CERTIFICATE-----\n    MIIBszCCAVmgAwIBAgIUQ3+Jh5b1\n    -----END CERTIFICATE-----\nkind: Secret\nmetadata:\n  \
         name: db\n  namespace: scene\n  resourceVersion: \"412\"\ntype: Opaque\n"
    )
}

/// The window with the editor of an object of a page, from the menu of its row.
fn editor(page: (&str, &str), yaml: &str, decoded: Option<Decoded>) -> Window {
    editor_in(egui::Theme::Dark, page, yaml, decoded)
}

fn editor_in(theme: egui::Theme, page: (&str, &str), yaml: &str, decoded: Option<Decoded>) -> Window {
    let (page, name) = page;
    let clusters =
        vec![allowed("ams-prod", &scene::<Deployment>()), with_secrets("fra-prod", &[secret("db", "Opaque", &[])])];
    let mut window = window_with(view(clusters), theme, SCALE);
    crate::fake::open_page(&mut window, page);
    until(&mut window, "the row", |window| window.query_all_by_role_and_label(Role::Label, name).next().is_some());
    window.get_by_role_and_label(Role::Label, name).click_secondary();
    window.run();
    window.get_by_label_contains("Edit manifest…").click();
    window.run();
    let request = editors(&window).pop().expect("the request of the editor");
    give(&mut window, &request, EditState::Done(text(yaml, decoded)));
    // The dock grows, so the text has room.
    crate::fake::grow_dock(&mut window, 200.0);
    window
}

#[test]
fn the_editor_with_colors_marks_a_search_a_secret_and_a_read_only_text() {
    let mut window = editor_in(egui::Theme::Light, ("Deployments", "web"), MANIFEST, None);
    checked(&mut window, "No error");
    save_clean(&mut window, "editor-colors-light");
    let mut window = editor(("Deployments", "web"), MANIFEST, None);
    checked(&mut window, "No error");
    save_clean(&mut window, "editor-colors");

    click_row(&mut window, "    tier: frontend");
    window.key_press(Key::Enter);
    window.event(Event::Text("on: yes".to_owned()));
    window.run();
    click_row(&mut window, "  replicas: 3");
    window.key_press(Key::Enter);
    window.event(Event::Text("replicas: 4".to_owned()));
    window.run();
    checked(&mut window, "1 error, 2 hints");
    save_clean(&mut window, "editor-marks");

    window.key_press_modifiers(Modifiers::COMMAND, Key::F);
    window.run();
    window.get_by_label("The text to find").type_text("web");
    window.key_press(Key::Enter);
    window.run();
    save_clean(&mut window, "editor-search");

    let mut window = editor(("Secrets", "db"), &secret_text(), Some(Decoded { texts: 2, binary: 1 }));
    checked(&mut window, "No error");
    save_clean(&mut window, "editor-secret");

    let lines: String = (1..=9_000).map(|number| format!("    line-{number:04}: value\n")).collect();
    let mut window =
        editor(("Deployments", "web"), &format!("apiVersion: apps/v1\nkind: Deployment\nspec:\n{lines}"), None);
    save_clean(&mut window, "editor-read-only");
}

/// The dialog of a save with its diff, the three versions of a conflict, and the tab of a
/// create with its dialog, its jobs and the dialog of a force.
#[test]
fn the_save_and_the_conflict_of_an_editor_and_the_tab_of_a_create() {
    use std::sync::Arc;

    use fresnel_core::app::{Remote, SaveState};
    use fresnel_core::manifest::merge::merge;

    use super::save::{answer, saves, version};

    let mut window = editor(("Deployments", "web"), MANIFEST, None);
    checked(&mut window, "No error");
    click_row(&mut window, "  replicas: 3");
    window.key_press(Key::Backspace);
    window.event(Event::Text("4".to_owned()));
    click_row(&mut window, "        image: nginx:1.27");
    window.key_press(Key::Backspace);
    window.event(Event::Text("8".to_owned()));
    window.run();
    window.get_by_label("Save the text").click();
    window.run();
    let check = saves(&window).pop().expect("the dry run");
    let after = MANIFEST.replace("replicas: 3", "replicas: 4").replace("nginx:1.27", "nginx:1.28");
    answer(&mut window, &check, SaveState::Checked(text(&after, None)));
    save_clean(&mut window, "editor-save-diff");

    // The dry run of the next save meets a change of the server.
    window.get_by_label("Cancel").click();
    window.run();
    window.get_by_label("Save the text").click();
    window.run();
    let check = saves(&window).pop().expect("the dry run");
    let server = MANIFEST.replace("replicas: 3", "replicas: 5").replace("tier: frontend", "tier: web");
    let server = version(&server.replace("\"8812\"", "\"42\""), "8840");
    let merged = merge(&check.base.text, &check.text, &server.text);
    answer(&mut window, &check, SaveState::Conflict(Arc::new(Remote { read: server, merge: merged })));
    save_clean(&mut window, "editor-conflict");

    let mut window = editor(("Deployments", "web"), MANIFEST, None);
    super::create::tab(&mut window);
    save_clean(&mut window, "yaml-tab");
    window.get_by_label("Apply the documents").click();
    window.run();
    save_clean(&mut window, "yaml-apply");
    window.get_by_label("Apply 2 objects").click();
    window.run();
    let orders = super::create::in_turn(&window);
    let field = r#".data.mode (conflict with "kubectl-create" using v1)"#;
    let message = r#"Apply failed with 1 conflict: conflict with "kubectl-create" using v1: .data.mode (HTTP 409)"#;
    let conflict = fresnel_core::CoreError::new(fresnel_core::net::ErrorClass::Conflict, message)
        .with_causes(vec![("FieldManagerConflict".to_owned(), field.to_owned())]);
    let states = [fresnel_core::ops::JobState::Failed(conflict), fresnel_core::ops::JobState::Done];
    super::create::jobs(&mut window, orders.into_iter().zip(states).collect());
    save_clean(&mut window, "yaml-jobs");
    window.get_by_label("Apply with force").click();
    window.run();
    save_clean(&mut window, "yaml-force");
}
