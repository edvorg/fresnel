//! The selection of the pane and its copy: a drag over rows, a double click on a line, and
//! a copy of each line of a large view, which a worker thread builds.

use std::time::{Duration, Instant};

use egui::{Event, Key, Modifiers, PointerButton};
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use fresnel_core::logs::SourceState;

use super::{copied, line, log_view, numbered, opened, rows, show, source, until_row};
use crate::fake::{Window, until};

const WEB: &str = "web-6888b778b6-75md7";

/// The places of the rows on the screen, from the top.
fn places(window: &Window) -> Vec<egui::Rect> {
    let mut places: Vec<egui::Rect> =
        window.query_all_by_role(egui::accesskit::Role::ListItem).map(|node| node.rect()).collect();
    places.sort_by(|a, b| a.top().total_cmp(&b.top()));
    places
}

fn button(pos: egui::Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
}

/// Draws frames until a frame puts a text on the clipboard. Gives that text.
fn until_copied(window: &mut Window) -> String {
    let started = Instant::now();
    loop {
        window.step();
        if let Some(text) = copied(window) {
            return text;
        }
        assert!(started.elapsed() < Duration::from_secs(30), "in time: a copy");
    }
}

#[test]
fn a_drag_selects_over_rows_and_the_copy_key_gives_the_lines_with_their_times() {
    let (mut window, request) = opened(WEB);
    let lines = vec![line(3.0, 0, "alpha one"), line(2.0, 0, "beta two"), line(1.0, 0, "gamma three")];
    show(&mut window, vec![log_view(&request, vec![source(WEB, "web", SourceState::Streaming)], lines)]);
    until_row(&mut window, "gamma three");
    let rows_at = places(&window);
    let (first, last) = (rows_at.first().expect("a row"), rows_at.last().expect("a row"));
    // From the start of the first row to the third character of the last one: the text
    // starts after the time of 13 columns.
    let column = window.ctx.fonts_mut(|fonts| fonts.glyph_width(&egui::FontId::monospace(12.0), 'x'));
    let start = first.left_center();
    let end = last.left_center() + egui::vec2((13.0 + 3.0) * column, 0.0);
    for event in [
        Event::PointerMoved(start),
        button(start, true),
        Event::PointerMoved(start + egui::vec2(0.0, 20.0)),
        Event::PointerMoved(end),
        button(end, false),
    ] {
        window.event(event);
        window.run();
    }
    let copy = window.get_by_role_and_label(egui::accesskit::Role::Button, "Copy");
    assert!(!copy.accesskit_node().is_disabled(), "a selection to copy");
    window.event(Event::Copy);
    window.step();
    let text = copied(&window).expect("a copy");
    assert_eq!(
        text,
        "2027-01-15T07:59:57.000000000Z alpha one\n2027-01-15T07:59:58.000000000Z beta two\n2027-01-15T07:59:59.000000000Z gam"
    );
    window.run();
    assert!(window.query_by_label_contains("Copied 3 lines (").is_some(), "{:?}", rows(&window));

    // A double click selects one line; the button copies it.
    let middle = places(&window).get(1).expect("a row").center();
    window.event(Event::PointerMoved(middle));
    for pressed in [true, false, true, false] {
        window.event(button(middle, pressed));
        window.step();
    }
    window.run();
    let copy = window.get_by_role_and_label(egui::accesskit::Role::Button, "Copy");
    assert!(!copy.accesskit_node().is_disabled(), "the double click selected a line");
    copy.click();
    let text = until_copied(&mut window);
    assert_eq!(text, "2027-01-15T07:59:58.000000000Z beta two");
}

#[test]
fn a_copy_of_each_line_of_a_large_view_runs_on_a_worker_thread() {
    let (mut window, request) = opened(WEB);
    show(&mut window, vec![log_view(&request, vec![source(WEB, "web", SourceState::Streaming)], numbered(20_000))]);
    until_row(&mut window, "line 19999");
    super::first_row(&window).click();
    window.key_press_modifiers(Modifiers::COMMAND, Key::A);
    window.run();
    window.event(Event::Copy);
    let text = until_copied(&mut window);
    assert_eq!(text.lines().count(), 20_000);
    assert!(text.starts_with("2027-01-15T07:56:40.000000000Z line 0000 of the log\n"), "{text:.80}");
    assert!(text.ends_with("line 19999 of the log"));
    until(&mut window, "the note of the copy", |window| {
        window.query_by_label_contains("Copied 20,000 lines").is_some()
    });
}

/// Extreme sizes: a hundred lines of 16 KiB each are a copy of 1.6 MB. The frame of the key
/// gives them to the worker thread, as a copy of many short lines.
#[test]
fn a_copy_of_some_long_lines_runs_on_a_worker_thread() {
    let (mut window, request) = opened(WEB);
    let long = |number: usize| line(100.0 - number as f64, 0, &format!("{number:03} {}", "x".repeat(16 * 1024)));
    show(
        &mut window,
        vec![log_view(&request, vec![source(WEB, "web", SourceState::Streaming)], (0..100).map(long).collect())],
    );
    until_row(&mut window, "099 xxx");
    super::first_row(&window).click();
    window.key_press_modifiers(Modifiers::COMMAND, Key::A);
    window.run();
    window.event(Event::Copy);
    window.step();
    assert_eq!(copied(&window).map(|text| text.len()), None, "the frame of the key builds no text");
    let text = until_copied(&mut window);
    assert_eq!(
        (text.lines().count(), text.len()),
        (100, 100 * (31 + 4 + 16 * 1024) + 99),
        "a time, a number and a line end"
    );
}
