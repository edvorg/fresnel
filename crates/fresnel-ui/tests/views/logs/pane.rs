//! The pane of a log view: follow, the wheel, the keys, wrap, the times and a long line.

use egui::accesskit::Role;
use egui::{Event, Key, Modifiers};
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use fresnel_core::logs::SourceState;

use super::{line, log_view, numbered, opened, pane, rows, show, source, until_row};
use crate::fake::{shows, until};
use crate::frame_time::wheel;

const WEB: &str = "web-6888b778b6-75md7";

fn checked(window: &crate::fake::Window, label: &str) -> bool {
    let node = window.get_by_role_and_label(Role::CheckBox, label);
    node.accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)
}

#[test]
fn follow_keeps_the_last_line_a_scroll_up_ends_it_and_a_scroll_to_the_end_starts_it_again() {
    let (mut window, request) = opened(WEB);
    let streaming = vec![source(WEB, "web", SourceState::Streaming)];
    show(&mut window, vec![log_view(&request, streaming.clone(), numbered(300))]);
    until_row(&mut window, "line 0299 of the log");
    assert!(checked(&window, "Follow"));
    let count = rows(&window).len();
    assert!(count > 3, "the pane has rows: {count}");

    super::first_row(&window).hover();
    window.event(wheel(120.0));
    window.run();
    assert!(!checked(&window, "Follow"), "a scroll up ends follow");
    let top = rows(&window);
    assert!(!top.iter().any(|row| row.contains("line 0299")), "the last line is below the pane");

    // New lines come: the pane stays where it is.
    show(&mut window, vec![log_view(&request, streaming.clone(), numbered(320))]);
    assert_eq!(rows(&window), top);

    // A scroll to the end starts follow again, and the new lines show.
    for _ in 0..40 {
        window.event(wheel(-200.0));
    }
    until(&mut window, "follow again", |window| checked(window, "Follow"));
    until_row(&mut window, "line 0319 of the log");

    // The keys of the pane: Home goes to the first line, End follows again.
    super::first_row(&window).click();
    window.key_press(Key::Home);
    window.run();
    assert!(rows(&window).first().is_some_and(|row| row.contains("line 0000")) && !checked(&window, "Follow"));
    window.key_press(Key::End);
    window.run();
    until_row(&mut window, "line 0319 of the log");
    assert!(checked(&window, "Follow"));
}

#[test]
fn wrap_cuts_a_long_line_into_rows_and_the_times_hide() {
    let (mut window, request) = opened(WEB);
    let long = format!("start {} end", "x".repeat(600));
    let lines = vec![line(2.0, 0, "short line"), line(1.0, 0, &long)];
    show(&mut window, vec![log_view(&request, vec![source(WEB, "web", SourceState::Streaming)], lines)]);
    until_row(&mut window, "short line");
    let one = rows(&window);
    assert_eq!(one.len(), 2, "one row for each line with no wrap");
    assert!(one.iter().any(|row| row.ends_with(" end")), "a row has the full line");

    window.get_by_role_and_label(Role::CheckBox, "Wrap").click();
    window.run();
    let wrapped = rows(&window);
    assert!(wrapped.len() > 3, "the long line takes rows: {wrapped:?}");
    assert!(wrapped.last().is_some_and(|row| row.ends_with("x end")), "its last part is in the last row");
    let width = pane(&window).width();
    assert!(wrapped.iter().all(|row| row.chars().count() as f32 * 6.0 < width + 120.0), "each row fits the pane");

    window.get_by_role_and_label(Role::CheckBox, "Times").click();
    window.run();
    assert!(rows(&window).first().is_some_and(|row| row == "short line"), "no time");
}

#[test]
fn a_line_past_the_laid_out_limit_shows_its_start_and_a_window_has_the_full_line() {
    let (mut window, request) = opened(WEB);
    let long = format!("{}THE END", "y".repeat(10_000));
    show(
        &mut window,
        vec![log_view(&request, vec![source(WEB, "web", SourceState::Streaming)], vec![line(1.0, 0, &long)])],
    );
    until(&mut window, "the button of the long line", |window| shows(window, "Show the full line: 10,007 bytes"));
    assert!(!rows(&window).iter().any(|row| row.contains("THE END")), "the pane lays out the start only");
    window.get_by_label("Show the full line: 10,007 bytes").click();
    window.run();
    assert!(shows(&window, "The full line: 10,007 bytes"));
    assert!(window.query_by_label_contains("yyyyTHE END").is_some(), "the window has the full line");
    window.get_by_label("Copy the line").click();
    window.step();
    assert_eq!(super::copied(&window).as_deref(), Some(long.as_str()));
}

#[test]
fn the_keys_of_the_pane_act_only_while_it_has_the_focus() {
    let (mut window, request) = opened(WEB);
    show(&mut window, vec![log_view(&request, vec![source(WEB, "web", SourceState::Streaming)], numbered(300))]);
    until_row(&mut window, "line 0299 of the log");
    window.key_press(Key::Home);
    window.run();
    assert!(checked(&window, "Follow"), "no focus: no key");
    super::first_row(&window).click();
    window.event(Event::Key {
        key: Key::PageUp,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    });
    window.run();
    assert!(!checked(&window, "Follow"), "a page up");
}
