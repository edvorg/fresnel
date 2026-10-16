//! The text of an editor: the marks of the check with their line and column, the hints that
//! never refuse a text, the search, F8, and the buttons of the undo.

use egui::accesskit::Role;
use egui::{Event, Key, Modifiers};
use egui_kittest::kittest::{NodeT as _, Queryable as _};

use super::{WEB, checked, click_row, edit, panel, rows, text};
use crate::fake::shows;

#[test]
fn an_error_of_the_parser_shows_with_its_line_and_column_and_a_hint_refuses_nothing() {
    let mut window = panel();
    edit(&mut window, text(WEB, None));
    checked(&mut window, "No error");

    // A second name in the metadata: the parser refuses the text.
    click_row(&mut window, "  name: web");
    window.key_press(Key::Enter);
    window.event(Event::Text("name: api".to_owned()));
    window.run();
    checked(&mut window, "1 error, 1 hint");
    assert!(shows(&window, "Line 5, column 3: Duplicate mapping key: name not allowed here."));
    let hint = "Line 5, column 9: The editor edits the object with the name web. The server refuses an other name.";
    assert!(shows(&window, hint));
    assert!(shows(&window, "Edit Deployment web •"));

    // Two steps of the undo take the line and its text back; `yes` as a value is a hint only.
    for _ in 0..2 {
        window.get_by_label("Undo").click();
        window.run();
    }
    checked(&mut window, "No error");
    assert!(!shows(&window, "Edit Deployment web •"), "the text of the read");
    click_row(&mut window, "  replicas: 2");
    window.key_press(Key::Enter);
    window.event(Event::Text("paused: yes".to_owned()));
    window.run();
    checked(&mut window, "No error, 1 hint");
    assert!(shows(&window, "Line 9, column 11: yes is the boolean true here, not a text. Quote it for a text."));

    // F8 goes to the line of the mark, and a click on a mark too.
    click_row(&mut window, "apiVersion: apps/v1");
    window.key_press(Key::F8);
    window.event(Event::Text("#".to_owned()));
    window.run();
    assert!(rows(&window).contains(&"  paused: #yes".to_owned()), "{:?}", rows(&window));
    checked(&mut window, "No error");
    window.key_press(Key::Backspace);
    window.run();
    checked(&mut window, "No error, 1 hint");
    window.get_by_label_contains("Line 9, column 11").click();
    window.event(Event::Text("!".to_owned()));
    window.run();
    assert!(rows(&window).contains(&"  paused: !yes".to_owned()), "{:?}", rows(&window));
    window.get_by_label("Revert to the text of the read").click();
    window.run();
    assert!(!shows(&window, "Edit Deployment web •"), "the text of the read again");
    window.get_by_label("Undo").click();
    window.run();
    assert!(rows(&window).contains(&"  paused: !yes".to_owned()), "the revert is one step of the undo");
    window.get_by_label("Redo").click();
    window.run();
    assert!(!shows(&window, "Edit Deployment web •"), "the revert again");
    assert!(window.get_by_label("Redo").accesskit_node().is_disabled());
}

#[test]
fn the_search_marks_each_hit_and_goes_to_the_next_with_enter_and_cmd_g() {
    let mut window = panel();
    edit(&mut window, text(WEB, None));
    click_row(&mut window, "kind: Deployment");
    window.key_press_modifiers(Modifiers::COMMAND, Key::F);
    window.run();
    assert!(window.get_by_label("The text to find").is_focused(), "Cmd+F in the text finds in it");
    window.get_by_label("The text to find").type_text("web");
    window.run();
    assert!(shows(&window, "1 of 2") || shows(&window, "2 hits"), "the count");
    window.key_press(Key::Enter);
    window.run();
    assert!(shows(&window, "1 of 2"));
    window.key_press(Key::Enter);
    window.run();
    assert!(shows(&window, "2 of 2"));
    window.key_press_modifiers(Modifiers::COMMAND, Key::G);
    window.run();
    assert!(shows(&window, "1 of 2"), "round the end");
    window.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::G);
    window.run();
    assert!(shows(&window, "2 of 2"));
    window.key_press(Key::Escape);
    window.run();
    assert!(window.query_by_label("The text to find").is_none(), "Escape closes the search");
    // The selection is the last hit: a typed text takes its place.
    window.event(Event::Text("api".to_owned()));
    window.run();
    assert!(rows(&window).contains(&"        name: api".to_owned()), "{:?}", rows(&window));
    window.key_press_modifiers(Modifiers::COMMAND, Key::F);
    window.run();
    assert!(shows(&window, "1 hit"), "the search keeps its text, and the cursor is past its hit");
    assert_eq!(window.query_all_by_role(Role::ListItem).count(), 14, "each line on the screen and the empty last one");
}
