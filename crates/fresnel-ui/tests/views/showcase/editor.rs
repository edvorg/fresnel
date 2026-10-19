//! The picture of the manifest editor: the Deployment checkout-api of prod-eu in the middle of
//! an edit, with a new image tag, a variable whose value YAML reads as a boolean, the hint
//! of the check about it, and a search.

use std::sync::Arc;

use egui::accesskit::Role;
use egui::{Event, Key, Modifiers};
use egui_kittest::kittest::Queryable as _;
use fresnel_core::app::EditState;
use fresnel_core::manifest::EditText;
use serde_json::json;

use super::objects::RESOURCE_VERSION;
use super::panels::{checkout_of, set_filter};
use super::stores::Stores;
use super::world::{CHECKOUT, PROD_EU};
use super::{save, window};
use crate::editor::{checked, click_row, editors, give};
use crate::fake::{Window, grow_dock, shows, until};
use crate::frame_time::wheel;

/// The dock grows this much, so the text shows the spec of the Deployment.
const EDITOR_DOCK: f32 = 270.0;
/// The generation of the spec of checkout-api: its revision 7 and two scales.
const GENERATION: u32 = 9;

/// checkout-api of prod-eu as a GET of the server gives it: the object of its store with the
/// defaults that the server sets, which no view of the world reads, in the YAML of the core.
fn served() -> String {
    let stores = Stores::of(&PROD_EU);
    let deployment =
        stores.deployments.iter().find(|deployment| deployment.metadata.name.as_deref() == Some(CHECKOUT.name));
    let mut object = serde_json::to_value(&**deployment.expect("the Deployment checkout-api")).expect("JSON");
    object["metadata"]["generation"] = GENERATION.into();
    object["status"]["observedGeneration"] = GENERATION.into();
    let spec = &mut object["spec"];
    (spec["progressDeadlineSeconds"], spec["revisionHistoryLimit"]) = (600.into(), 10.into());
    let pod = &mut spec["template"]["spec"];
    for (field, value) in [
        ("dnsPolicy", json!("ClusterFirst")),
        ("restartPolicy", json!("Always")),
        ("schedulerName", json!("default-scheduler")),
        ("securityContext", json!({})),
        ("terminationGracePeriodSeconds", json!(30)),
    ] {
        pod[field] = value;
    }
    for container in pod["containers"].as_array_mut().into_iter().flatten() {
        container["imagePullPolicy"] = "IfNotPresent".into();
        container["terminationMessagePath"] = "/dev/termination-log".into();
        container["terminationMessagePolicy"] = "File".into();
    }
    serde_saphyr::to_string(&object).expect("YAML")
}

/// Types text at the end of a row of the editor: a key press for each new line, so each one
/// gets the indent of the editor.
fn type_after(window: &mut Window, row: &str, lines: &[&str]) {
    click_row(window, row);
    for line in lines {
        window.key_press(Key::Enter);
        window.event(Event::Text((*line).to_owned()));
        window.run();
    }
}

/// The editor of checkout-api, scrolled to its spec: the release 1.42.4 is in the image tag,
/// and a new variable has the value `on`, which YAML reads as the boolean true. The search
/// finds the version in the text.
#[test]
fn an_edit_of_a_manifest() {
    let mut window = window(egui::Theme::Dark);
    checkout_of(&mut window, PROD_EU.name);
    set_filter(&mut window, CHECKOUT.name, "3 of 19 Deployments");
    window.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::Y);
    window.run();
    let request = editors(&window).pop().expect("the request of the editor");
    let text = served();
    let read = EditText {
        lines: text.lines().count(),
        text: text.into(),
        version: Some(RESOURCE_VERSION.to_owned()),
        decoded: None,
    };
    give(&mut window, &request, EditState::Done(Arc::new(read)));
    grow_dock(&mut window, EDITOR_DOCK);
    checked(&mut window, "No error");
    // The wheel brings the spec to the top of the text.
    let row = |window: &Window, text: &str| window.get_by_role_and_label(Role::ListItem, text).rect();
    let first = row(&window, "apiVersion: apps/v1");
    window.hover_at(first.center());
    window.event(wheel(first.top() - row(&window, "spec:").top()));
    window.run();
    let image = format!("      - image: {}", CHECKOUT.image);
    click_row(&mut window, &image);
    window.key_press(Key::Backspace);
    window.event(Event::Text("4".to_owned()));
    window.run();
    type_after(&mut window, "        name: checkout-api", &["env:", "- name: CHECKOUT_SPLIT_PAYMENTS", "value: on"]);
    checked(&mut window, "No error, 1 hint");
    window.key_press_modifiers(Modifiers::COMMAND, Key::F);
    window.run();
    window.get_by_label("The text to find").type_text("1.42");
    window.key_press(Key::Enter);
    until(&mut window, "the hit", |window| shows(window, "1 of 1"));
    save(&mut window, "yaml-editor");
}
