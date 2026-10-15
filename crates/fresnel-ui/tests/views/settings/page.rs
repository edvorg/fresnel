//! The settings view: a control for each setting, a save of the changes as one command,
//! each problem of the file with its line and column, and no save of a file that the app
//! cannot read.

use std::path::PathBuf;
use std::sync::Arc;

use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use fresnel_core::app::{Command, CoreView};
use fresnel_core::config::{Edit, EditValue, FileRead, FileState, Problem, SaveState};

use super::with_config;
use crate::fake::{Window, connected, live_status, open_page, shows, view, window};

/// The view of a core that read this settings file.
fn read(text: &str) -> CoreView {
    let mut view = with_config(view(vec![connected("ams-prod", live_status())]), text);
    let problems = Vec::new();
    let path = PathBuf::from("/Users/tester/Library/Application Support/fresnel/config.toml");
    view.config_file = Arc::new(FileState { path, read: FileRead::Read { problems, used: true }, save: None });
    view
}

fn edits(window: &Window) -> Vec<Vec<Edit>> {
    let sent = window.state().sent.iter();
    sent.filter_map(|command| match command {
        Command::EditSettings(edits) => Some(edits.clone()),
        _ => None,
    })
    .collect()
}

fn save_is_off(window: &Window) -> bool {
    window.get_by_label("Save the settings").accesskit_node().is_disabled()
}

/// Replaces the text of a field, as a user who scrolls to it, selects it and types.
pub(crate) fn retype(window: &mut Window, field: &str, text: &str) {
    window.get_by_role_and_label(Role::TextInput, field).scroll_to_me();
    window.run();
    window.get_by_role_and_label(Role::TextInput, field).click();
    window.run();
    window.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    window.get_by_role_and_label(Role::TextInput, field).type_text(text);
    window.run();
}

/// The text of a field.
fn value(window: &Window, field: &str) -> Option<String> {
    window.get_by_role_and_label(Role::TextInput, field).value()
}

#[test]
fn a_change_of_each_form_goes_to_the_core_as_one_save_and_a_bad_number_keeps_save_off() {
    let mut window = window(read("version = 1\n[table]\nrow_height = 28\n"));
    open_page(&mut window, "Settings");
    assert!(shows(&window, "Read with no problem."));
    assert!(shows(&window, "/Users/tester/Library/Application Support/fresnel/config.toml"));
    assert!(save_is_off(&window), "no change yet");
    assert_eq!(value(&window, "Height of a row").as_deref(), Some("28"), "the value of the file");

    retype(&mut window, "Height of a row", "99");
    assert!(shows(&window, "a whole number from 20 to 40"));
    assert!(shows(&window, "1 field to correct first") && save_is_off(&window));
    retype(&mut window, "Height of a row", "32");
    window.get_by_role_and_label(Role::CheckBox, "Each cluster is read-only").click();
    window.run();
    retype(&mut window, "Alias of ams-prod", "AMS");
    assert!(shows(&window, "3 changes to save."));
    window.get_by_label("Save the settings").click();
    window.run();
    let mut sent = edits(&window);
    sent.iter_mut().for_each(|edits| edits.sort_by(|a, b| a.path.cmp(&b.path)));
    assert_eq!(
        sent,
        [vec![
            Edit::new(&["clusters", "ams-prod", "alias"], EditValue::Text("AMS".to_owned())),
            Edit::new(&["safety", "read_only"], EditValue::Flag(true)),
            Edit::new(&["table", "row_height"], EditValue::Number(32)),
        ]]
    );
    assert!(shows(&window, "The app writes the file…") && save_is_off(&window), "one save at a time");

    // The core ends the save: the draft goes, and the controls show the file again.
    let mut after = read("version = 1\n[table]\nrow_height = 32\n");
    Arc::make_mut(&mut after.config_file).save = Some(SaveState { number: 1, result: Some(Ok(())) });
    window.state_mut().view = after;
    window.run();
    assert!(shows(&window, "Saved into the file.") && shows(&window, "No change."));
    assert_eq!(value(&window, "Height of a row").as_deref(), Some("32"));
}

#[test]
fn a_key_that_the_file_sets_gets_its_default_and_a_refused_save_keeps_the_draft() {
    let mut window = window(read("version = 1\n[table]\nrow_height = 28\n"));
    open_page(&mut window, "Settings");
    window.get_by_label("Default of Height of a row").click();
    window.run();
    window.get_by_label("Save the settings").click();
    window.run();
    assert_eq!(edits(&window), [vec![Edit::new(&["table", "row_height"], EditValue::Remove)]]);

    let mut refused = read("version = 1\n[table]\nrow_height = 28\n");
    let error = "cannot replace the file: Permission denied (os error 13)".to_owned();
    Arc::make_mut(&mut refused.config_file).save = Some(SaveState { number: 1, result: Some(Err(error.clone())) });
    window.state_mut().view = refused;
    window.run();
    assert!(shows(&window, &format!("Not saved: {error}")));
    assert!(shows(&window, "1 change to save.") && !save_is_off(&window), "the draft stays for a new try");
}

#[test]
fn a_file_that_does_not_parse_shows_its_place_and_the_view_saves_nothing() {
    let mut broken = read("version = 1\n");
    let problem = Problem { line: 3, column: 9, text: "unclosed table, expected `]`".to_owned() };
    Arc::make_mut(&mut broken.config_file).read = FileRead::Read { problems: vec![problem], used: false };
    let mut window = window(broken);
    open_page(&mut window, "Settings");
    assert!(shows(&window, "The app uses no setting of the file:"));
    assert!(shows(&window, "line 3, column 9: unclosed table, expected `]`"));
    assert!(shows(
        &window,
        "The file has a problem at line 3, column 9: unclosed table, expected `]`. Correct it in an editor first."
    ));
    assert!(save_is_off(&window));
    assert!(
        window.get_by_role_and_label(Role::TextInput, "Height of a row").accesskit_node().is_disabled(),
        "no change of a file that the app cannot read"
    );
    assert_eq!(
        window
            .query_all_by_role(Role::Button)
            .filter(|button| button.accesskit_node().label().as_deref() == Some("Save the settings"))
            .count(),
        1
    );
}

/// The view refuses each text that the reader of the file would drop, so the file gets the
/// value that the user sees: a namespace of the settings is a name of Kubernetes.
#[test]
fn a_text_that_is_no_name_of_a_namespace_keeps_save_off() {
    let mut window = window(read("version = 1\n"));
    open_page(&mut window, "Settings");
    retype(&mut window, "Default namespace of ams-prod", "Shop");
    assert!(shows(&window, "`Shop` is no name of a namespace: small letters, digits and -"));
    assert!(save_is_off(&window));
    crate::keys::typed(&mut window, "Favorite namespaces of ams-prod", "shop\nkube_system");
    assert!(shows(&window, "`kube_system` is no name of a namespace: small letters, digits and -"));
    retype(&mut window, "Default namespace of ams-prod", "shop");
    crate::keys::typed(&mut window, "Favorite namespaces of ams-prod", "shop\nkube-system");
    assert!(shows(&window, "2 changes to save.") && !save_is_off(&window));
}
