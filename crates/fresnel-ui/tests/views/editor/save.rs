//! The save of an editor (plan section 12.9): Save asks for a dry run, the dialog shows its
//! diff with the target and sends the text that the user saw, the answer is the new read; a
//! conflict merges or shows the three versions; a read-only cluster and the confirm rule
//! keep the save off.

use std::sync::Arc;

use egui::Event;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use fresnel_core::app::{Command, EditSave, Remote, SaveState, SaveView, Stage};
use fresnel_core::manifest::EditText;
use fresnel_core::manifest::merge::merge;

use super::{WEB, checked, click_row, edit, panel, rows, text};
use crate::fake::{Window, shows};
use crate::settings::with_config;

/// The saves of the editors that the views sent.
pub(crate) fn saves(window: &Window) -> Vec<EditSave> {
    let sent = window.state().sent.iter();
    sent.filter_map(|command| match command {
        Command::SaveEditor(save) => Some(save.clone()),
        _ => None,
    })
    .collect()
}

/// Gives the fake core the state of a save of the editor.
pub(crate) fn answer(window: &mut Window, save: &EditSave, state: SaveState) {
    put(window, save, state);
    window.run();
}

/// Puts the state of a save of the editor into the fake core, with no frame.
pub(crate) fn put(window: &mut Window, save: &EditSave, state: SaveState) {
    let read = window.state_mut().view.edits.first_mut().expect("the read of the editor");
    let mut changed = (**read).clone();
    changed.save = Some(SaveView { save: save.clone(), state });
    *read = Arc::new(changed);
}

/// A text of the editor with this version.
pub(crate) fn version(yaml: &str, version: &str) -> Arc<EditText> {
    let yaml = yaml.replace("resourceVersion: \"42\"", &format!("resourceVersion: \"{version}\""));
    Arc::new(EditText { version: Some(version.to_owned()), ..(*text(&yaml, None)).clone() })
}

/// Changes the count of pods in the text of the editor of `web` to `count`.
pub(crate) fn replicas(window: &mut Window, count: &str) {
    click_row(window, "  replicas: 2");
    window.key_press(egui::Key::Backspace);
    window.event(Event::Text(count.to_owned()));
    window.run();
}

/// The editor of `web` with a change of its count of pods, and the dry run of its save.
fn checked_save(window: &mut Window) -> EditSave {
    edit(window, text(WEB, None));
    checked(window, "No error");
    let button = window.get_by_label("Save the text");
    assert!(button.accesskit_node().is_disabled(), "no change, no save");
    replicas(window, "3");
    window.get_by_label("Save the text").click();
    window.run();
    saves(window).pop().expect("the dry run")
}

#[test]
fn a_save_shows_the_diff_of_its_dry_run_and_sends_the_text_that_the_user_saw() {
    let mut window = panel();
    let check = checked_save(&mut window);
    assert_eq!((check.number, check.stage, check.base.version.as_deref()), (1, Stage::Check, Some("42")));
    assert!(check.text.contains("  replicas: 3\n") && check.editor.target.key.name.as_ref() == "web");
    answer(&mut window, &check, SaveState::Pending);
    assert!(shows(&window, "The server checks the text in a dry run…"));

    answer(&mut window, &check, SaveState::Checked(text(&WEB.replace("replicas: 2", "replicas: 3"), None)));
    assert!(shows(&window, "Save this Deployment?"));
    assert!(shows(&window, "    8       -   replicas: 2") && shows(&window, "          8 +   replicas: 3"));
    assert!(shows(&window, "    7     7   spec:"), "the lines around the change");
    window.get_by_label("Save the manifest").click();
    window.run();
    let send = saves(&window).pop().expect("the replace");
    assert_eq!((send.number, send.stage), (2, Stage::Send));
    assert!(send.text == check.text && send.base == check.base, "the text and the read of the dialog");

    // The text waits for the answer of its save.
    answer(&mut window, &send, SaveState::Pending);
    assert!(shows(&window, "The save is on its way…"));
    click_row(&mut window, "  replicas: 3");
    window.event(Event::Text("0".to_owned()));
    window.run();
    assert!(rows(&window).contains(&"  replicas: 3".to_owned()), "a held text takes no key");

    // A version after the one of the store: the panel shows the change as pending.
    let saved = version(&WEB.replace("replicas: 2", "replicas: 3"), "99999943");
    answer(&mut window, &send, SaveState::Saved(saved));
    assert!(shows(&window, "Saved: the object has version 99999943 now."));
    assert!(shows(&window, "version 99999943 · 13 lines"), "the answer is the new read");
    assert!(shows(&window, "Change pending: the server stored a change, and the watch did not bring it yet."));
    assert!(!shows(&window, "Edit Deployment web •"), "no change after the save: {:?}", rows(&window));
    replicas_again(&mut window);
}

/// A change after a save goes with the version of its answer.
fn replicas_again(window: &mut Window) {
    click_row(window, "  replicas: 3");
    window.key_press(egui::Key::Backspace);
    window.event(Event::Text("4".to_owned()));
    window.run();
    window.get_by_label("Save the text").click();
    window.run();
    let next = saves(window).pop().expect("a next dry run");
    assert_eq!((next.number, next.base.version.as_deref()), (3, Some("99999943")));
}

#[test]
fn the_confirm_rule_and_a_read_only_cluster_keep_the_save_off() {
    let mut window = panel();
    let settings = "version = 1\n[clusters.ams-prod]\nconfirm = \"type-name\"\n";
    window.state_mut().view = with_config(window.state().view.clone(), settings);
    let check = checked_save(&mut window);
    answer(&mut window, &check, SaveState::Checked(text(&WEB.replace("replicas: 2", "replicas: 3"), None)));
    assert!(window.get_by_label("Save the manifest").accesskit_node().is_disabled());
    window.get_by_label("Name of the cluster").click();
    window.run();
    window.get_by_label("Name of the cluster").type_text("ams-prod");
    window.run();
    assert!(!window.get_by_label("Save the manifest").accesskit_node().is_disabled(), "the name of the cluster");

    let read_only = "version = 1\n[clusters.ams-prod]\nread_only = true\n";
    window.state_mut().view = with_config(window.state().view.clone(), read_only);
    window.run();
    assert!(window.get_by_label("Save the manifest").accesskit_node().is_disabled(), "the dialog reads the settings");
    assert!(shows(&window, "ams-prod is read-only in the settings: the app makes no change in it."));
    window.get_by_label("Cancel").click();
    window.run();
    assert!(window.get_by_label("Save the text").accesskit_node().is_disabled(), "and the bar too");
}

#[test]
fn a_conflict_merges_the_changes_of_other_lines_and_the_user_saves_again() {
    let mut window = panel();
    let check = checked_save(&mut window);
    let server = version(&WEB.replace("nginx:1.27", "nginx:1.28"), "44");
    let merged = merge(&check.base.text, &check.text, &server.text);
    answer(&mut window, &check, SaveState::Conflict(Arc::new(Remote { read: server, merge: merged })));
    let note = "The object changed on the server since the read: version 44 now. Its changes are in the text \
                with yours: check it and save again.";
    assert!(shows(&window, note));
    let shown = rows(&window);
    assert!(
        shown.contains(&"  replicas: 3".to_owned()) && shown.contains(&"      - image: nginx:1.28".to_owned()),
        "{shown:?}"
    );
    window.get_by_label("Save the text").click();
    window.run();
    let again = saves(&window).pop().expect("a dry run of the merge");
    assert_eq!((again.stage, again.base.version.as_deref()), (Stage::Check, Some("44")));
}

#[test]
fn changes_of_the_same_lines_show_three_versions_and_the_chosen_side_goes_into_the_text() {
    let mut window = panel();
    let check = checked_save(&mut window);
    let server = version(&WEB.replace("replicas: 2", "replicas: 5"), "44");
    let merged = merge(&check.base.text, &check.text, &server.text);
    answer(&mut window, &check, SaveState::Conflict(Arc::new(Remote { read: server, merge: merged })));
    for text in ["The read, version 42", "Yours", "The server now, version 44", "  replicas: 2", "  replicas: 5"] {
        assert!(shows(&window, text), "{text}");
    }
    window.get_by_label("Take the server's at place 1").click();
    window.run();
    window.get_by_label("Use the merge in the editor").click();
    window.run();
    assert!(rows(&window).contains(&"  replicas: 5".to_owned()), "{:?}", rows(&window));
    assert!(shows(&window, "version 44 · 13 lines"), "the new read of the server");
    assert!(!shows(&window, "Edit Deployment web •"), "the side of the server is its text");
}

/// The dialog of a save takes the keys and the clicks: the text stays the text that the
/// dry run checked, so the answer of the save loses no edit.
#[test]
fn the_text_takes_no_key_while_the_dialog_of_its_save_is_open() {
    let mut window = panel();
    let check = checked_save(&mut window);
    answer(&mut window, &check, SaveState::Checked(text(&WEB.replace("replicas: 2", "replicas: 3"), None)));
    window.event(Event::Text("Z".to_owned()));
    window.run();
    click_row(&mut window, "  replicas: 3");
    window.event(Event::Text("Q".to_owned()));
    window.key_press(egui::Key::Backspace);
    window.run();
    let shown = rows(&window);
    assert!(shown.contains(&"  replicas: 3".to_owned()) && !shown.iter().any(|row| row.contains(['Z', 'Q'])));
    assert!(shows(&window, "Save this Deployment?"), "the dialog stays");
}
