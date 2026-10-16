//! The editor has a limit of 8,000 lines (plan section 12.9): a larger manifest opens
//! read-only, with virtual rows, its search and a copy, and a key does not change it.

use egui::{Event, Key, Modifiers};
use egui_kittest::kittest::Queryable as _;

use super::{click_row, edit, panel, rows, text};
use crate::fake::shows;

#[test]
fn a_manifest_of_more_than_8_000_lines_opens_read_only_with_its_search() {
    let lines: String = (1..=8_001).map(|number| format!("    key{number:04}: value\n")).collect();
    let yaml = format!("apiVersion: apps/v1\nkind: Deployment\nspec:\n{lines}");
    let mut window = panel();
    edit(&mut window, text(&yaml, None));
    let note = "The manifest has 8,004 lines, more than the 8,000 of the editor, so it is read-only.";
    assert!(shows(&window, note));
    assert!(
        window.query_by_label("Undo").is_none() && window.query_by_label("Revert to the text of the read").is_none()
    );
    assert!(rows(&window).len() < 60, "virtual rows: {}", rows(&window).len());

    click_row(&mut window, "kind: Deployment");
    window.event(Event::Text("x".to_owned()));
    window.key_press(Key::Enter);
    window.run();
    assert!(rows(&window).contains(&"kind: Deployment".to_owned()), "no change");
    assert!(shows(&window, "The text is read-only."));
    window.key_press_modifiers(Modifiers::COMMAND, Key::F);
    window.run();
    window.get_by_label("The text to find").type_text("key8001");
    window.key_press(Key::Enter);
    window.run();
    assert!(
        shows(&window, "1 of 1") && rows(&window).contains(&"    key8001: value".to_owned()),
        "{:?}",
        rows(&window)
    );
}
