//! The keys of the window (plan section 11.5): the overlay of the keys, and the focus of
//! the keys in the sidebar, the table, the panel and the dock.

use egui::accesskit::Role;
use egui::{Key, Modifiers};
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use fresnel_core::commands::COMMAND_KEY;

use crate::data::{live, pods};
use crate::fake::{Window, shows, until, view, window};

fn focused(window: &Window, role: Role, label: &str) -> bool {
    window.get_by_role_and_label(role, label).accesskit_node().is_focused()
}

fn press(window: &mut Window, modifiers: Modifiers, key: Key) {
    window.key_press_modifiers(modifiers, key);
    window.run();
}

#[test]
fn the_question_mark_shows_the_keys_and_a_text_field_keeps_it_as_text() {
    let mut window = window(view(vec![live("ams-prod", &pods())]));
    press(&mut window, Modifiers::SHIFT, Key::Questionmark);
    assert!(shows(&window, "Keys") && shows(&window, "Quick search") && shows(&window, "Focus the table"));
    assert!(shows(&window, "Restart…") || shows(&window, "Restart"), "an action with its key");
    assert!(shows(&window, "Move the cursor of a table; Shift selects a range"));
    // The keys of the areas name the command key of the system: Ctrl on Windows.
    assert!(shows(&window, &format!("{COMMAND_KEY}+A, Escape")), "the command key of the system");
    press(&mut window, Modifiers::SHIFT, Key::Questionmark);
    assert!(!shows(&window, "Focus the table"), "? closes the overlay");

    window.get_by_role(Role::TextInput).click();
    window.run();
    window.get_by_role(Role::TextInput).type_text("?");
    press(&mut window, Modifiers::SHIFT, Key::Questionmark);
    assert_eq!(window.get_by_role(Role::TextInput).value().as_deref(), Some("?"), "the filter has the text");
    assert!(!shows(&window, "Focus the table"), "no overlay while the user types");
}

#[test]
fn the_focus_keys_move_through_the_sidebar_the_table_the_panel_and_the_dock() {
    let mut window = window(view(vec![live("ams-prod", &pods())]));
    press(&mut window, Modifiers::COMMAND, Key::Num1);
    assert!(focused(&window, Role::RadioButton, "Pods"), "the entry of the open page");
    press(&mut window, Modifiers::NONE, Key::ArrowDown);
    assert!(focused(&window, Role::RadioButton, "Deployments"), "the arrows move the focus");
    press(&mut window, Modifiers::NONE, Key::Enter);
    let open = |window: &Window, page: &str| {
        let node = window.get_by_role_and_label(Role::RadioButton, page);
        node.accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)
    };
    assert!(open(&window, "Deployments"), "Enter opens the page of the focus");

    press(&mut window, Modifiers::NONE, Key::ArrowUp);
    press(&mut window, Modifiers::NONE, Key::Space);
    assert!(open(&window, "Pods"), "Space too");
    // The new page has no row until its worker thread gives them: an arrow then moves nothing.
    until(&mut window, "the rows of the Pod table", |window| shows(window, "25 pods"));
    press(&mut window, Modifiers::COMMAND, Key::Num2);
    press(&mut window, Modifiers::NONE, Key::ArrowDown);
    assert!(shows(&window, "Overview"), "the first row and its panel");
    press(&mut window, Modifiers::COMMAND, Key::Num3);
    assert!(focused(&window, Role::Button, "Overview"), "the tab of the panel");
    press(&mut window, Modifiers::NONE, Key::ArrowRight);
    assert!(focused(&window, Role::Button, "Manifest"));
    press(&mut window, Modifiers::COMMAND, Key::Num4);
    assert!(focused(&window, Role::Button, "Journal"), "the open tab of the dock");
    press(&mut window, Modifiers::NONE, Key::F6);
    assert!(focused(&window, Role::RadioButton, "Pods"), "F6 goes from the dock to the sidebar");
    press(&mut window, Modifiers::SHIFT, Key::F6);
    assert!(focused(&window, Role::Button, "Journal"), "Shift+F6 goes back");
}
