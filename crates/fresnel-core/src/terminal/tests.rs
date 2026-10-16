//! The terminal on recorded output of busybox 1.36.1, the version of the test pods, in a TTY
//! of 80x24 (`fixtures/terminal`), and on escape sequences.

use super::*;
use crate::config::Rgb;

const VI: &[u8] = include_bytes!("../../fixtures/terminal/vi.bin");
const TOP: &[u8] = include_bytes!("../../fixtures/terminal/top.bin");
const SHELL: &[u8] = include_bytes!("../../fixtures/terminal/shell.bin");

fn terminal(columns: u16, rows: u16) -> Terminal {
    Terminal::new(Options { size: GridSize::new(columns, rows), scrollback: 100 })
}

fn copied(terminal: &Terminal) -> Frame {
    let mut frame = Frame::default();
    assert!(terminal.frame(&mut frame, &Palette::DARK));
    frame
}

fn rows(terminal: &Terminal) -> Vec<String> {
    let frame = copied(terminal);
    (0..usize::from(frame.size.rows)).map(|row| frame.text(row)).collect()
}

/// The input that waits for the session, all pieces.
fn input(terminal: &Terminal) -> Vec<u8> {
    std::iter::from_fn(|| terminal.take_input()).flatten().collect()
}

fn ansi(index: usize) -> Rgb {
    Palette::DARK.ansi[index]
}

#[test]
fn busybox_vi_shows_on_the_alternate_screen_and_the_shell_screen_comes_back_after_it() {
    let terminal = terminal(80, 24);
    terminal.feed(b"$ vi /tmp/notes.txt\r\n");
    let quit = VI.windows(8).position(|window| window == b"\x1b[?1049l").expect("the end of vi");
    terminal.feed(&VI[..quit]);
    let frame = copied(&terminal);
    assert!(frame.modes.alt_screen);
    assert_eq!(frame.text(0), "line 9 of the notes");
    assert_eq!(frame.text(21), "line 30 of the notes");
    assert_eq!(frame.text(22), "a new line from the test");
    assert_eq!(frame.cursor.map(|cursor| (cursor.row, cursor.column)), Some((23, 0)), "on the line of commands");
    assert_eq!(frame.history, 0, "the alternate screen has no history");

    terminal.feed(&VI[quit..]);
    let after = rows(&terminal);
    assert!(!copied(&terminal).modes.alt_screen);
    assert_eq!(after.first().map(String::as_str), Some("$ vi /tmp/notes.txt"));
    assert!(after.iter().all(|row| !row.contains("of the notes")), "{after:?}");
}

#[test]
fn busybox_top_draws_its_screen_again_in_place_with_an_inverse_header() {
    let terminal = terminal(80, 24);
    terminal.feed(TOP);
    let frame = copied(&terminal);
    assert!(frame.text(0).starts_with("Mem: "), "{}", frame.text(0));
    assert!(frame.text(2).starts_with("Load average: "));
    assert_eq!(frame.text(3), "  PID  PPID USER     STAT   VSZ %VSZ CPU %CPU COMMAND");
    assert!(frame.text(4).ends_with("yes"), "{}", frame.text(4));
    let header = frame.row(3).get(..frame.text(3).len()).expect("the cells of the header");
    assert!(header.iter().all(|cell| cell.bg == Palette::DARK.foreground && cell.fg == Palette::DARK.background));
    assert!(frame.row(4).iter().all(|cell| cell.bg == Palette::DARK.background));
    assert_eq!(frame.history, 0, "three screens in place leave no history");
}

#[test]
fn the_shell_gets_its_answers_and_its_colors_show() {
    let terminal = terminal(80, 24);
    terminal.feed(SHELL);
    // ash asks for the cursor position after each prompt.
    let answers = String::from_utf8(input(&terminal)).expect("text");
    assert!(answers.starts_with("\x1b[1;3R"), "{answers:?}");
    let frame = copied(&terminal);
    let row = (0..24).find(|row| frame.text(*row).ends_with(" bin")).expect("the row of bin");
    let bin = frame.row(row).iter().rev().find(|cell| cell.c == 'b').expect("b");
    assert_eq!(bin.fg, ansi(4));
    assert!(bin.flags.contains(CellFlags::BOLD));
    let colors = (0..24).find(|row| frame.text(*row).starts_with("bold red")).expect("the printf row");
    let cells = frame.row(colors);
    assert_eq!(cells[0].fg, ansi(1));
    assert_eq!(cells[9].fg, Rgb([255, 135, 0]), "color 208");
    assert_eq!(cells[16].bg, Rgb([10, 20, 30]), "a true color");
    assert_eq!(cells[15].bg, Palette::DARK.background);
}

#[test]
fn wide_characters_take_two_cells_and_marks_stay_on_their_character() {
    let terminal = terminal(20, 2);
    terminal.feed("中a😀e\u{301}│".as_bytes());
    let frame = copied(&terminal);
    let cells = frame.row(0);
    assert_eq!((cells[0].c, cells[0].flags.contains(CellFlags::WIDE)), ('中', true));
    assert!(cells[1].flags.contains(CellFlags::SPACER));
    assert_eq!(cells[2].c, 'a');
    assert!(cells[3].flags.contains(CellFlags::WIDE) && cells[4].flags.contains(CellFlags::SPACER));
    assert_eq!((cells[5].c, frame.marks(0, 5)), ('e', Some("\u{301}")));
    assert!(cells[5].flags.contains(CellFlags::MARKS));
    assert_eq!(frame.text(0), "中a😀e\u{301}│");
}

#[test]
fn a_long_title_is_cut_and_the_output_after_it_shows() {
    let terminal = terminal(40, 4);
    let mut output = b"\x1b]0;".to_vec();
    output.extend(std::iter::repeat_n(b't', 10 * 1024 * 1024));
    output.extend(b"\x07\x07after the title");
    terminal.feed(&output);
    assert_eq!(terminal.title().map(|title| title.len()), Some(events::TITLE_LIMIT));
    assert_eq!(rows(&terminal)[0], "after the title");
    terminal.feed(b"\x1b]2;web-1\x07");
    assert_eq!(terminal.title().as_deref(), Some("web-1"));
}

#[test]
fn the_answers_tell_the_size_and_the_colors_of_the_user() {
    let terminal = terminal(80, 24);
    terminal.resize(GridSize::new(100, 30), (16, 31));
    assert_eq!(terminal.take_size(), Some(GridSize::new(100, 30)));
    assert_eq!(terminal.take_size(), None, "one time for each change");
    // A request for the clipboard (OSC 52) gets no answer: the remote program reads nothing
    // of the user.
    terminal.feed(b"\x1b[14t\x1b]52;c;?\x07\x1b]11;?\x07");
    let answers = String::from_utf8(input(&terminal)).expect("text");
    assert_eq!(answers, "\x1b[4;930;1600t\x1b]11;rgb:1111/1313/1717\x07");
}

#[test]
fn a_resize_flows_a_wrapped_line_again() {
    let terminal = terminal(40, 8);
    terminal.feed(b"0123456789012345678901234567890123456789ABCDEFGHIJ");
    assert_eq!(rows(&terminal)[1], "ABCDEFGHIJ");
    terminal.resize(GridSize::new(60, 8), (8, 16));
    assert_eq!(rows(&terminal)[0], "0123456789012345678901234567890123456789ABCDEFGHIJ");
    assert_eq!(terminal.size(), GridSize::new(60, 8));
}

#[test]
fn the_history_scrolls_and_input_brings_the_view_to_the_bottom() {
    let terminal = terminal(20, 4);
    for line in 1..=30 {
        terminal.feed(format!("line {line}\r\n").as_bytes());
    }
    assert_eq!(rows(&terminal)[0], "line 28");
    terminal.scroll(Scroll::Lines(5));
    let frame = copied(&terminal);
    assert_eq!((frame.offset, frame.history, frame.text(0).as_str()), (5, 27, "line 23"));
    assert!(frame.cursor.is_none(), "the cursor is below the view");
    terminal.send(b"x").expect("room");
    assert_eq!(copied(&terminal).offset, 0);
    terminal.set_scrollback(10);
    assert_eq!(copied(&terminal).history, 10);
}

#[test]
fn a_selection_gives_its_text_and_marks_its_cells() {
    let terminal = terminal(20, 3);
    terminal.feed(b"hello world\r\nsecond line");
    terminal.select(SelectKind::Cells, Spot { row: 0, column: 6, right: false });
    terminal.select_to(Spot { row: 1, column: 5, right: true });
    assert_eq!(terminal.selection_text().as_deref(), Some("world\nsecond"));
    let frame = copied(&terminal);
    assert!(frame.row(0)[6].flags.contains(CellFlags::SELECTED));
    assert!(!frame.row(0)[5].flags.contains(CellFlags::SELECTED));
    terminal.select(SelectKind::Words, Spot { row: 1, column: 8, right: false });
    assert_eq!(terminal.selection_text().as_deref(), Some("line"));
    assert!(terminal.clear_selection() && !terminal.clear_selection());
    assert_eq!(terminal.selection_text(), None);
}

#[test]
fn input_waits_in_pieces_up_to_its_limit_and_a_paste_follows_the_mode() {
    let terminal = terminal(20, 3);
    let mega = "y".repeat(1024 * 1024);
    assert_eq!(terminal.paste(&mega), Ok(mega.len()));
    let pieces: Vec<Vec<u8>> = std::iter::from_fn(|| terminal.take_input()).collect();
    assert_eq!(pieces.len(), 64);
    assert!(pieces.iter().all(|piece| piece.len() == PIECE));
    assert_eq!(pieces.concat(), mega.as_bytes(), "each byte, in its place");

    terminal.feed(b"\x1b[?2004h");
    assert_eq!(terminal.paste("a\nb"), Ok(15));
    assert_eq!(input(&terminal), b"\x1b[200~a\nb\x1b[201~");

    let full = vec![b'z'; INPUT_LIMIT];
    terminal.send(&full).expect("room for the limit");
    assert_eq!(terminal.send(b"!"), Err(Refused::Full { bytes: 1, room: 0 }));
    assert_eq!(
        (terminal.waiting_input(), terminal.drop_input(), terminal.waiting_input()),
        (INPUT_LIMIT, INPUT_LIMIT, 0)
    );
}

/// An attach to a container with no stdin: a key would go nowhere with no sign, so the
/// terminal refuses each key and paste with the reason, and counts each answer of its parser.
#[test]
fn a_program_with_no_input_refuses_each_key_and_counts_each_answer() {
    let terminal = terminal(20, 3);
    terminal.send(b"typed before the stream").expect("room");
    terminal.refuse_input();
    assert_eq!(terminal.waiting_input(), 0, "the keys before go nowhere either");
    assert_eq!(terminal.send(b"ls\r"), Err(Refused::NoInput));
    assert_eq!(terminal.paste("ls\n"), Err(Refused::NoInput));
    assert_eq!(Refused::NoInput.to_string(), "the program has no input: its container has no stdin in its spec");
    terminal.feed(b"\x1b[6n");
    assert_eq!((terminal.waiting_input(), terminal.refused_answers()), (0, 6), "the answer `ESC [1;1R`");
}

#[test]
fn a_synchronized_update_shows_after_its_end_or_after_end_sync() {
    let terminal = terminal(20, 2);
    terminal.feed(b"\x1b[?2026hhidden");
    assert_eq!(rows(&terminal)[0], "");
    assert!(terminal.sync_deadline().is_some());
    terminal.end_sync();
    assert_eq!((rows(&terminal)[0].as_str(), terminal.sync_deadline()), ("hidden", None));
}

#[test]
fn a_frame_copies_the_screen_only_after_a_change() {
    let terminal = terminal(20, 2);
    let mut frame = Frame::default();
    assert!(terminal.frame(&mut frame, &Palette::DARK));
    assert!(!terminal.frame(&mut frame, &Palette::DARK));
    assert!(terminal.frame(&mut frame, &Palette::LIGHT), "a new palette");
    terminal.feed(b"x");
    assert!(terminal.frame(&mut frame, &Palette::LIGHT));
    assert_eq!(frame.row(0)[0].fg, Palette::LIGHT.foreground);
}

#[test]
fn the_cursor_has_the_shape_that_the_program_asks_for() {
    let terminal = terminal(20, 2);
    let shape = |terminal: &Terminal| copied(terminal).cursor.map(|cursor| cursor.shape);
    assert_eq!(shape(&terminal), Some(CursorShape::Block));
    terminal.feed(b"\x1b[6 q");
    assert_eq!(shape(&terminal), Some(CursorShape::Beam));
    terminal.feed(b"\x1b[4 q");
    assert_eq!(shape(&terminal), Some(CursorShape::Underline));
    terminal.feed(b"\x1b[?25l");
    assert_eq!(shape(&terminal), None);
}

#[test]
fn the_modes_of_the_pointer_and_the_keys_reach_the_frame() {
    let terminal = terminal(20, 2);
    terminal.feed(b"\x1b[?1h\x1b[?1002h\x1b[?1006h\x1b[?1004h");
    let modes = copied(&terminal).modes;
    assert!(modes.app_cursor && modes.focus_reports);
    assert_eq!((modes.mouse, modes.mouse_encoding), (MouseMode::Drag, MouseEncoding::Sgr));
}

#[tokio::test(start_paused = true)]
async fn the_session_wakes_for_keys_for_a_new_size_and_for_an_answer() {
    let terminal = terminal(20, 4);
    let woke = |terminal: &Terminal| {
        let terminal = terminal.clone();
        async move { tokio::time::timeout(std::time::Duration::from_secs(1), terminal.wanted()).await.is_ok() }
    };
    assert!(!woke(&terminal).await, "nothing waits");
    terminal.send(b"ls").expect("room");
    assert!(woke(&terminal).await);
    terminal.resize(GridSize::new(30, 4), (8, 16));
    assert!(woke(&terminal).await);
    terminal.feed(b"\x1b[6n");
    assert!(woke(&terminal).await);
    assert_eq!(input(&terminal), b"ls\x1b[1;1R");
    assert!(!woke(&terminal).await, "each wake one time");
}

#[test]
fn the_default_history_is_the_default_of_the_settings() {
    let lines = crate::config::Config::default().number(crate::config::table::Setting::TerminalScrollback);
    assert_eq!(Options::default().scrollback as u64, lines);
}

#[test]
fn a_repeat_of_the_remote_program_has_a_limit_and_repeats_no_mark() {
    let short = terminal(20, 2);
    short.feed(b"ab\x1b[3b");
    assert_eq!(rows(&short)[0], "abbbb");
    let long = Terminal::new(Options { size: GridSize::new(20, 2), scrollback: 1000 });
    long.feed(b"x\x1b[65535b");
    let cells = 1 + usize::from(guard::REPEAT_LIMIT);
    // The 2 lines of the screen and the history above them, with 20 cells each.
    assert_eq!(copied(&long).history + 2, cells.div_ceil(20), "the limit, not 65,536 cells");

    let marks = Terminal::new(Options { size: GridSize::new(20, 2), scrollback: 10 });
    let mut output = "e\u{301}".as_bytes().to_vec();
    (0..40).for_each(|_| output.extend(b"\x1b[65535b"));
    (0..10).for_each(|_| output.extend([b"\x80".as_slice(), "\u{302}".repeat(16).as_bytes()].concat()));
    marks.feed(&output);
    let frame = copied(&marks);
    assert_eq!(frame.marks(0, 0).map(|marks| marks.chars().count()), Some(guard::MARKS_LIMIT));
}

#[test]
fn the_history_keeps_no_link_of_the_remote_program() {
    let terminal = terminal(20, 2);
    terminal.feed(b"\x1b]8;;https://example.test/a\x07link\x1b]8;;\x07 text");
    assert_eq!(rows(&terminal)[0], "link text");
    use alacritty_terminal::index::{Column, Line, Point};
    let screen = terminal.shared.screen.lock();
    let cell = &screen.term.grid()[Point::new(Line(0), Column(0))];
    assert_eq!(cell.hyperlink(), None, "a link of each cell kept 4 KiB for each cell of the history");
}
