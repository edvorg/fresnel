//! The screen of a terminal: its size, the recorded programs, the history and the settings.

use egui::{Event, Key, Modifiers};
use egui_kittest::kittest::Queryable as _;
use fresnel_core::terminal::GridSize;

use super::{SHELL, TOP, VI, focus, input, opened, rows, terminal, with_terminals};
use crate::data::{live, pods};
use crate::fake::{Window, grow_dock, in_one_frame, press, until, view};
use crate::settings::with_config;

/// A dock with room for the 80 x 24 of the recordings.
fn tall(window: &mut Window) {
    grow_dock(window, 240.0);
}

#[test]
fn the_terminal_gets_the_size_of_its_area_and_a_new_size_when_the_dock_grows() {
    let web = terminal(1, "web-1");
    let mut window = opened(std::slice::from_ref(&web));
    let first = web.terminal.take_size().expect("the size of the area");
    assert!(first.columns > 100 && first.rows > 10, "{first:?}");
    assert_eq!(web.terminal.size(), first);
    tall(&mut window);
    let taller = web.terminal.take_size().expect("a new size");
    assert_eq!(taller.columns, first.columns);
    assert!(taller.rows >= first.rows + 15, "{first:?} to {taller:?}");
    window.run();
    assert_eq!(web.terminal.take_size(), None, "one time for each change");
}

#[test]
fn busybox_vi_shows_on_the_alternate_screen_and_top_draws_its_screen_again() {
    let web = terminal(1, "web-1");
    let mut window = opened(std::slice::from_ref(&web));
    tall(&mut window);
    let quit = VI.windows(8).position(|window| window == b"\x1b[?1049l").expect("the end of vi");
    web.terminal.feed(b"$ vi /tmp/notes.txt\r\n");
    web.terminal.feed(VI.get(..quit).expect("vi"));
    window.run();
    let shown = rows(&window);
    assert_eq!(shown.first().map(String::as_str), Some("line 9 of the notes"), "{shown:?}");
    assert_eq!(shown.get(22).map(String::as_str), Some("a new line from the test"));
    web.terminal.feed(VI.get(quit..).expect("vi"));
    window.run();
    assert_eq!(rows(&window).first().map(String::as_str), Some("$ vi /tmp/notes.txt"), "the screen of the shell");

    web.terminal.feed(b"\x1b[H\x1b[J");
    web.terminal.feed(TOP);
    window.run();
    let shown = rows(&window);
    assert!(shown.first().is_some_and(|row| row.starts_with("Mem: ")), "{shown:?}");
    assert_eq!(shown.get(3).map(String::as_str), Some("  PID  PPID USER     STAT   VSZ %VSZ CPU %CPU COMMAND"));
    assert!(shown.iter().filter(|row| row.contains("sleep 30")).count() == 5);
}

#[test]
fn the_wheel_moves_through_the_history_and_a_key_brings_the_view_back() {
    let web = terminal(1, "web-1");
    let mut window = opened(std::slice::from_ref(&web));
    for line in 1..=300 {
        web.terminal.feed(format!("line {line}\r\n").as_bytes());
    }
    web.terminal.feed(SHELL);
    window.run();
    let bottom = rows(&window);
    let area = window.get_by_label("Terminal").rect();
    in_one_frame(&mut window, [Event::PointerMoved(area.center()), crate::frame_time::wheel(200.0)]);
    until(&mut window, "an older part of the history", |window| rows(window) != bottom);
    assert!(rows(&window).iter().any(|row| row.starts_with("line ")));
    assert!(window.query_by_label_contains("up in a history of").is_some(), "the line tells the place");
    window.get_by_label("To the end").click();
    window.run();
    assert_eq!(rows(&window), bottom);
    in_one_frame(&mut window, [Event::PointerMoved(area.center()), crate::frame_time::wheel(200.0)]);
    until(&mut window, "the history again", |window| rows(window) != bottom);

    focus(&mut window);
    assert!(input(&web.terminal).starts_with(b"\x1b[19;3R"), "the answers to the cursor requests of ash");
    in_one_frame(&mut window, [Event::Text("x".into())]);
    window.run();
    assert_eq!(rows(&window), bottom, "the input goes to the bottom, where it shows");
    assert_eq!(input(&web.terminal), b"x");
}

#[test]
fn the_settings_give_the_font_size_and_option_as_meta() {
    let web = terminal(1, "web-1");
    let mut window = opened(std::slice::from_ref(&web));
    let normal = web.terminal.take_size().expect("a size");
    let text = "version = 1\n[terminal]\nfont_size = 26\noption_as_meta = true\n";
    let view = with_config(with_terminals(view(vec![live("ams-prod", &pods())]), std::slice::from_ref(&web)), text);
    window.state_mut().view = view;
    window.run();
    let large: GridSize = web.terminal.take_size().expect("a new size");
    assert!(large.columns <= normal.columns / 2 + 1 && large.rows <= normal.rows / 2 + 1, "{normal:?}, {large:?}");

    focus(&mut window);
    in_one_frame(&mut window, press(Modifiers::ALT, Key::B).into_iter().chain([Event::Text("∫".into())]));
    assert_eq!(input(&web.terminal), b"\x1bb", "Option as Meta drops the composed character");
}
