//! The keys, the paste, the selection and the pointer of a terminal.

use egui::{Event, Key, Modifiers, PointerButton, Pos2};
use egui_kittest::kittest::Queryable as _;
use fresnel_core::terminal::{INPUT_LIMIT, PIECE, Terminal};

use super::{focus, input, opened, terminal};
use crate::fake::{Window, in_one_frame, press, shows};

/// A place in a row of the terminal on the screen: `column` 6.25 is in the left half of the
/// cell 6, so a selection from there takes the cell.
fn cell(window: &Window, terminal: &Terminal, (row, column): (usize, f32)) -> Pos2 {
    let area = window.get_by_label("Terminal").rect();
    let rows =
        window.query_all_by_role(egui::accesskit::Role::ListItem).filter(|node| area.contains(node.rect().center()));
    let row_rect = rows.map(|node| node.rect()).min_by(|a, b| a.top().total_cmp(&b.top())).expect("a row");
    let width = row_rect.width() / f32::from(terminal.size().columns);
    area.min + egui::vec2(column * width, (row as f32 + 0.5) * row_rect.height())
}

fn button(pos: Pos2, pressed: bool, modifiers: Modifiers) -> Event {
    Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers }
}

#[test]
fn the_keys_go_to_the_terminal_and_only_those_with_cmd_to_the_app() {
    let web = terminal(1, "web-1");
    let mut window = opened(std::slice::from_ref(&web));
    focus(&mut window);
    assert_eq!(input(&web.terminal), b"", "a click sends nothing");
    in_one_frame(&mut window, [Event::Text("ls -l".into())]);
    in_one_frame(&mut window, press(Modifiers::NONE, Key::Enter));
    in_one_frame(&mut window, press(Modifiers::CTRL, Key::C));
    in_one_frame(&mut window, press(Modifiers::NONE, Key::ArrowUp));
    in_one_frame(&mut window, press(Modifiers::NONE, Key::Tab));
    in_one_frame(&mut window, press(Modifiers::NONE, Key::F6));
    in_one_frame(&mut window, [Event::Text("?".into())]);
    in_one_frame(&mut window, press(Modifiers::SHIFT, Key::Questionmark));
    assert_eq!(input(&web.terminal), b"ls -l\r\x03\x1b[A\t\x1b[17~?");
    assert!(!shows(&window, "Focus the table"), "? is a key of the terminal, not the overlay of the keys");
    in_one_frame(&mut window, [Event::Text("z".into())]);
    assert_eq!(input(&web.terminal), b"z", "Tab, F6 and the arrows keep the focus in the terminal");

    // Cmd on macOS belongs to the app. On Windows Ctrl is the key of the app, and the
    // terminal takes it: Ctrl+K deletes to the end of a line of the shell.
    let cmd = Modifiers { mac_cmd: true, command: true, ..Modifiers::NONE };
    in_one_frame(&mut window, press(cmd, Key::K));
    window.run();
    assert!(window.query_by_label("Search text").is_some(), "Cmd+K opens the quick search");
    assert_eq!(input(&web.terminal), b"");
}

#[test]
fn a_paste_goes_whole_and_a_paste_above_64_kib_waits_for_the_user() {
    let web = terminal(1, "web-1");
    let mut window = opened(std::slice::from_ref(&web));
    focus(&mut window);
    in_one_frame(&mut window, [Event::Paste("ls\nexit\n".into())]);
    assert_eq!(input(&web.terminal), b"ls\rexit\r", "Enter is a carriage return");

    let mebibyte = "y".repeat(1024 * 1024);
    in_one_frame(&mut window, [Event::Paste(mebibyte.clone())]);
    window.run();
    let question = "Paste 1,048,576 bytes into the terminal? They go in pieces of 16,384 bytes.";
    assert!(shows(&window, question) && web.terminal.waiting_input() == 0);
    window.get_by_label("Paste").click();
    window.run();
    assert!(shows(&window, "1,048,576 bytes of input wait for the program"));
    let pieces: Vec<Vec<u8>> = std::iter::from_fn(|| web.terminal.take_input()).collect();
    assert!(pieces.len() == 64 && pieces.iter().all(|piece| piece.len() == PIECE));
    assert_eq!(pieces.concat(), mebibyte.as_bytes(), "each byte in its place");

    in_one_frame(&mut window, [Event::Paste(mebibyte)]);
    window.run();
    window.get_by_label("Cancel").click();
    window.run();
    assert!(!shows(&window, question) && web.terminal.waiting_input() == 0);
    in_one_frame(&mut window, [Event::Text("q".into())]);
    assert_eq!(input(&web.terminal), b"q", "the terminal has the focus again");
}

#[test]
fn a_drag_selects_text_a_copy_takes_it_and_a_click_ends_it() {
    let web = terminal(1, "web-1");
    let mut window = opened(std::slice::from_ref(&web));
    web.terminal.feed(b"hello world\r\nsecond line");
    window.run();
    let (from, to) = (cell(&window, &web.terminal, (0, 6.25)), cell(&window, &web.terminal, (1, 5.75)));
    for event in [Event::PointerMoved(from), button(from, true, Modifiers::NONE)] {
        in_one_frame(&mut window, [event]);
    }
    for step in 1..=4 {
        in_one_frame(&mut window, [Event::PointerMoved(from + (to - from) * (step as f32 / 4.0))]);
    }
    in_one_frame(&mut window, [button(to, false, Modifiers::NONE)]);
    assert_eq!(web.terminal.selection_text().as_deref(), Some("world\nsecond"));
    in_one_frame(&mut window, [Event::Copy]);
    let copied = window
        .output()
        .platform_output
        .commands
        .iter()
        .any(|command| matches!(command, egui::OutputCommand::CopyText(text) if text == "world\nsecond"));
    assert!(copied, "the clipboard has the selection");
    assert_eq!(input(&web.terminal), b"", "a copy sends no byte");
    window.get_by_label("Terminal").click();
    window.run();
    assert_eq!(web.terminal.selection_text(), None);
}

#[test]
fn a_program_that_asks_for_the_pointer_gets_reports_and_shift_keeps_the_selection() {
    let web = terminal(1, "web-1");
    let mut window = opened(std::slice::from_ref(&web));
    web.terminal.feed(b"\x1b[?1000h\x1b[?1006h\x1b[?1004h");
    window.run();
    let at = cell(&window, &web.terminal, (2, 4.25));
    in_one_frame(&mut window, [Event::PointerMoved(at), button(at, true, Modifiers::NONE)]);
    in_one_frame(&mut window, [button(at, false, Modifiers::NONE)]);
    window.run();
    assert_eq!(input(&web.terminal), b"\x1b[I\x1b[<0;5;3M\x1b[<0;5;3m", "the focus and the click");
    in_one_frame(&mut window, [crate::frame_time::wheel(60.0)]);
    for _ in 0..10 {
        window.step();
    }
    let wheel = String::from_utf8(input(&web.terminal)).expect("text");
    assert!(wheel.starts_with("\x1b[<64;5;3M"), "{wheel:?}");

    web.terminal.feed(b"\r\n\r\nsome text here");
    window.run();
    in_one_frame(&mut window, [Event::ModifiersChanged(Modifiers::SHIFT), button(at, true, Modifiers::SHIFT)]);
    let end = cell(&window, &web.terminal, (2, 8.75));
    for step in 1..=3 {
        in_one_frame(&mut window, [Event::PointerMoved(at + (end - at) * (step as f32 / 3.0))]);
    }
    in_one_frame(&mut window, [button(end, false, Modifiers::SHIFT), Event::ModifiersChanged(Modifiers::NONE)]);
    assert_eq!(input(&web.terminal), b"", "Shift keeps the pointer with the user");
    assert_eq!(web.terminal.selection_text().as_deref(), Some(" text"), "the cells 4 to 8 of the row");
}

#[test]
fn input_that_does_not_fit_gives_a_note_until_some_input_goes() {
    let web = terminal(1, "web-1");
    let mut window = opened(std::slice::from_ref(&web));
    focus(&mut window);
    web.terminal.send(&vec![b'z'; INPUT_LIMIT]).expect("room for the limit");
    in_one_frame(&mut window, [Event::Text("q".into())]);
    window.run();
    let note = format!("The input did not go: the input has 0 free bytes of {INPUT_LIMIT}, too few for 1.");
    assert!(shows(&window, &note));
    assert!(shows(&window, "8,388,608 bytes of input wait for the program"));
    window.get_by_label("Drop the input").click();
    window.run();
    assert_eq!(web.terminal.waiting_input(), 0);
    focus(&mut window);
    in_one_frame(&mut window, [Event::Text("q".into())]);
    window.run();
    assert!(!shows(&window, &note) && input(&web.terminal) == b"q");
}
