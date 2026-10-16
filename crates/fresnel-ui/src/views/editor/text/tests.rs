//! The text widget in a harness with no window: the rows on the screen, the keys, the
//! pointer, the undo and the copy.

use egui::{Event, Key, Modifiers, OutputCommand};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};

use super::*;

fn harness(text: &str, limit: usize) -> Harness<'static, TextArea> {
    let area = TextArea::new(text, limit, true);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 300.0))
        .build_ui_state(|ui, area: &mut TextArea| area.show(ui, Overlay::default()), area);
    harness.run();
    harness
}

/// The text of each row on the screen, from the top.
fn rows(harness: &Harness<'_, TextArea>) -> Vec<String> {
    let rows = harness.query_all_by_role(egui::accesskit::Role::ListItem);
    let mut rows: Vec<(f32, String)> =
        rows.map(|row| (row.rect().top(), row.accesskit_node().label().unwrap_or_default())).collect();
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    rows.into_iter().map(|(_, text)| text).collect()
}

/// A click at the end of the row with this text.
fn click_row(harness: &mut Harness<'_, TextArea>, text: &str) {
    harness.get_by_label(text).click();
    harness.run();
}

#[test]
fn only_the_lines_on_the_screen_have_rows_and_a_move_to_the_end_shows_the_last_line() {
    let text: String = (1..=8000).map(|number| format!("key{number:04}: value\n")).collect();
    let mut harness = harness(&text, 8000);
    let shown = rows(&harness);
    assert!(shown.first().is_some_and(|row| row == "key0001: value"), "{shown:?}");
    assert!(shown.len() < 40, "{} rows", shown.len());
    click_row(&mut harness, "key0002: value");
    harness.key_press_modifiers(Modifiers::COMMAND, Key::End);
    harness.run();
    assert!(rows(&harness).iter().any(|row| row == "key8000: value"), "the cursor shows");
    assert_eq!(harness.state().cursor(), Pos::new(8000, 0));
}

#[test]
fn typed_keys_edit_the_text_in_yaml_and_the_undo_takes_them_back() {
    let mut harness = harness("spec:\n  replicas: 1\n", 100);
    click_row(&mut harness, "spec:");
    harness.key_press(Key::Enter);
    harness.event(Event::Text("paused".to_owned()));
    harness.event(Event::Text(":".to_owned()));
    harness.key_press(Key::Tab);
    harness.event(Event::Text("true".to_owned()));
    harness.run();
    assert_eq!(harness.state().text(), "spec:\n  paused: true\n  replicas: 1\n", "Enter indents, Tab gives a space");
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    harness.run();
    assert_eq!(harness.state().text(), "spec:\n  paused: \n  replicas: 1\n");
    harness.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
    harness.run();
    assert!(harness.state().text().contains("paused: true"), "the redo");
    assert!(rows(&harness).iter().any(|row| row == "  paused: true"));
}

#[test]
fn a_selection_of_all_gives_its_text_to_a_copy_and_a_cut_removes_it() {
    let mut harness = harness("a: 1\nb: 2\n", 100);
    click_row(&mut harness, "a: 1");
    harness.key_press_modifiers(Modifiers::COMMAND, Key::A);
    harness.event(Event::Copy);
    // The copy is an output of the frame of the event.
    harness.step();
    let copied = harness
        .output()
        .platform_output
        .commands
        .iter()
        .any(|command| matches!(command, OutputCommand::CopyText(text) if text == "a: 1\nb: 2\n"));
    assert!(copied, "{:?}", harness.output().platform_output.commands);
    harness.event(Event::Cut);
    harness.run();
    assert_eq!(harness.state().text(), "");
    harness.event(Event::Paste("c: 3\r\n".to_owned()));
    harness.run();
    assert_eq!(harness.state().text(), "c: 3\n");
}

#[test]
fn a_line_of_megabytes_shows_its_part_around_the_screen() {
    let text = format!("raw: {}\nnext: 1\n", "x".repeat(1_000_000));
    let harness = harness(&text, 100);
    let long = rows(&harness).into_iter().find(|row| row.starts_with("raw: ")).unwrap_or_default();
    assert!(long.len() > 50 && long.len() < 2000, "{} bytes of the line", long.len());
    assert!(rows(&harness).iter().any(|row| row == "next: 1"));
}

#[test]
fn a_paste_past_the_line_limit_does_not_go_in_and_the_note_tells_why() {
    let mut harness = harness("a: 1\n", 3);
    click_row(&mut harness, "a: 1");
    harness.event(Event::Paste("\nb: 2\nc: 3\nd: 4".to_owned()));
    harness.run();
    assert_eq!(harness.state().text(), "a: 1\n");
    assert_eq!(harness.state().note(), Some("The editor holds 3 lines at most, so the text did not change."));
}
