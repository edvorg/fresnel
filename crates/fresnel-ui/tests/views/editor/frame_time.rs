//! The time of a frame of the editor with 8,000 lines (plan section 17, risk 5): a frame
//! takes 16 ms or less while the user types, deletes, undoes, scrolls and finds, and while
//! the worker thread checks the text of each key press. The save dialog of such a text opens
//! with no long frame: a worker computes its diff.
//!
//! The text widget of egui took 8 to 11 ms for a frame with no input and up to 47 ms after
//! a key press with this text. The limit holds in a release build: `cargo test --release
//! --package fresnel-ui --test views -- frame_time --nocapture`. A debug build runs the
//! same steps and checks no time.

use egui::{Event, Key, Modifiers};
use egui_kittest::kittest::Queryable as _;
use fresnel_core::app::SaveState;
use k8s_openapi::api::apps::v1::Deployment;

use super::save::{put, saves};
use super::{checked, click_row, edit, rows, text};
use crate::data::scene;
use crate::fake::{shows, view, window_with_no_gpu};
use crate::frame_time::{Step, wheel};
use crate::timing::{Pass, timed};
use crate::workloads::scene::{allowed, open};

/// A Deployment of 7,990 lines, near the limit of 8,000: a container with 3,989 variables.
fn manifest() -> String {
    let mut text =
        "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: web\n  namespace: scene\nspec:\n  template:\n    \
                    spec:\n      containers:\n      - name: web\n        env:\n"
            .to_owned();
    for number in 0..3_989 {
        text.push_str(&format!("        - name: VARIABLE_{number:04}\n          value: \"the value of {number}\"\n"));
    }
    text.push_str("  replicas: 2\n");
    text
}

fn key(key: Key, modifiers: Modifiers) -> Event {
    Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
}

#[test]
fn a_frame_of_the_editor_with_8_000_lines_stays_in_the_frame_limit_during_typing_scroll_and_search() {
    timed(|pass| {
        let yaml = manifest();
        assert_eq!(yaml.lines().count(), 7_990);
        let mut window = window_with_no_gpu(view(vec![allowed("ams-prod", &scene::<Deployment>())]), 2.0);
        open(&mut window, "Deployments", "web");
        edit(&mut window, text(&yaml, None));
        let mut warm = Step::new("editor, the first frames");
        (0..20).for_each(|_| warm.frame(&mut window));
        checked(&mut window, "No error");

        // A key press with a check of the full text on the worker thread for each one.
        let mut typing = Step::new("editor, typing");
        click_row(&mut window, "kind: Deployment");
        typing.events(&mut window, (0..30).map(|_| key(Key::PageDown, Modifiers::NONE)));
        typing
            .events(&mut window, "# a comment of the user".chars().map(|character| Event::Text(character.to_string())));
        typing.events(&mut window, (0..5).map(|_| key(Key::Enter, Modifiers::NONE)));
        typing.events(&mut window, (0..10).map(|_| key(Key::Backspace, Modifiers::NONE)));
        typing.events(&mut window, (0..5).map(|_| key(Key::Z, Modifiers::COMMAND)));
        typing.events(&mut window, [key(Key::End, Modifiers::COMMAND), Event::Text("x".to_owned())]);
        (0..30).for_each(|_| typing.frame(&mut window));
        assert!(rows(&window).contains(&"x".to_owned()), "the end of the text: {:?}", rows(&window).last());
        typing.check(pass);

        let mut scroll = Step::new("editor, scroll");
        let middle = window.get_by_label("The text of the editor").rect().center();
        scroll.events(&mut window, [Event::PointerMoved(middle)]);
        scroll.events(&mut window, (0..200).map(|_| wheel(240.0)));
        scroll.check(pass);

        // Each second line has a hit, and the move to each one scrolls.
        let mut search = Step::new("editor, search");
        search.events(&mut window, [key(Key::F, Modifiers::COMMAND)]);
        search.events(&mut window, "VARIABLE_".chars().map(|character| Event::Text(character.to_string())));
        search.events(&mut window, (0..20).map(|_| key(Key::G, Modifiers::COMMAND)));
        search.until(&mut window, "the count of the hits", |window| shows(window, "20 of 3,989"));
        search.check(pass);
    });
}

/// A Deployment with a note of `lines` equal lines of 400 characters between a first and a
/// last line.
fn notes(lines: usize, (first, last): (&str, &str)) -> String {
    let line = format!("      {}\n", "a line of the notes, the same in each line; ".repeat(9));
    let notes = format!("      {first}\n{}      {last}\n", line.repeat(lines));
    format!(
        "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  annotations:\n    notes: |\n{notes}  name: web\n  \
         namespace: scene\nspec:\n  replicas: 2\n"
    )
}

/// A Deployment with a note of `lines` lines of about `width` characters that differ only at
/// their end, by `mark`.
fn long_notes((lines, width): (usize, usize), mark: char) -> String {
    let start = "the same start of each line; ".repeat(width / 29);
    let notes: String = (0..lines).map(|line| format!("      {start}{mark}{line:04}\n")).collect();
    format!(
        "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  annotations:\n    notes: |\n{notes}  name: web\n  \
         namespace: scene\nspec:\n  replicas: 2\n"
    )
}

/// Opens the save dialog of an editor of `old` whose dry run gives `new`, and checks its frames
/// until the diff shows the removed line that starts with `removed`.
fn save_dialog(pass: &mut Pass, step: &'static str, (old, new): (&str, &str), removed: &str) {
    let mut window = window_with_no_gpu(view(vec![allowed("ams-prod", &scene::<Deployment>())]), 2.0);
    open(&mut window, "Deployments", "web");
    edit(&mut window, text(old, None));
    checked(&mut window, "No error");
    click_row(&mut window, "kind: Deployment");
    window.event(key(Key::Enter, Modifiers::NONE));
    window.event(Event::Text("# saved".to_owned()));
    window.run();
    window.get_by_label("Save the text").click();
    window.run();
    let check = saves(&window).pop().expect("the dry run");
    put(&mut window, &check, SaveState::Checked(text(new, None)));

    let mut dialog = Step::new(step);
    let row = format!("-       {removed}");
    dialog.until(&mut window, "the rows of the diff", |window| {
        shows(window, "Save this Deployment?") && window.query_all_by_label_contains(&row).next().is_some()
    });
    dialog.check(pass);
}

/// The longest diff of 8,000 lines: a save that removes 1,000 of 7,000 equal lines and changes
/// the lines around them. Each of the 1,000 steps of Myers' diff follows the equal lines on
/// each of its diagonals, and the diff then shows a new text. In the frame, it took 38 ms.
#[test]
fn the_save_dialog_of_a_diff_of_8_000_lines_stays_in_the_frame_limit() {
    let (old, new) = (notes(7_000, ("first", "last")), notes(6_000, ("start", "end")));
    timed(|pass| save_dialog(pass, "editor, the save dialog", (&old, &new), "first"));
}

/// A diff of 1,000 lines in all, as many as a diff in the frame has, of lines of 6,000
/// characters: a save changes the end of each one. Each of the 1,000 steps of Myers' diff
/// compares the long equal start of two lines on each diagonal. In the frame, it took 41 ms.
#[test]
fn the_save_dialog_of_a_diff_of_long_lines_stays_in_the_frame_limit() {
    let (old, new) = (long_notes((491, 6_000), 'a'), long_notes((491, 6_000), 'b'));
    assert_eq!(old.lines().count() + new.lines().count(), 1_000);
    timed(|pass| save_dialog(pass, "editor, the save dialog of long lines", (&old, &new), "the same start"));
}

/// The largest diff that the frame computes: 1,000 lines in all, of 128 characters that a
/// save changes at their end, within the 128 KiB of its limit.
#[test]
fn the_save_dialog_of_the_largest_diff_in_the_frame_stays_in_the_frame_limit() {
    let (old, new) = (long_notes((491, 128), 'a'), long_notes((491, 128), 'b'));
    assert_eq!(old.lines().count() + new.lines().count(), 1_000);
    assert!(old.len() + new.len() <= 128 * 1024, "{} bytes", old.len() + new.len());
    let step = "editor, the save dialog of the largest diff in the frame";
    timed(|pass| save_dialog(pass, step, (&old, &new), "the same start"));
}
