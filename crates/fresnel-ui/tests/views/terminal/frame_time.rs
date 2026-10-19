//! The time of a frame of a terminal of 200 x 60 (plan section 12.8): while 10 MB of output
//! arrive on a second thread, as from a session, and with a full redraw of the screen before
//! each frame. The limit and the release build are those of `crate::frame_time`.

use std::sync::atomic::{AtomicBool, Ordering};

use fresnel_core::terminal::GridSize;

use super::{rows, terminal, title, with_terminals};
use crate::data::{live, pods};
use crate::fake::{grow_dock, open_tab, view, window_with_no_gpu};
use crate::frame_time::Step;

/// The output of a busy log: a time, a level in its color and a request. Ends with `last`.
fn log_output(bytes: usize, last: &str) -> Vec<u8> {
    let levels = [("32", "INFO"), ("33", "WARN"), ("31", "ERROR"), ("36", "DEBUG")];
    let mut output = String::with_capacity(bytes + 200);
    let mut number = 0_usize;
    while output.len() < bytes {
        let (color, level) = levels[number % levels.len()];
        let (second, milli, took) = (number / 1000 % 60, number % 1000, number % 97);
        output.push_str(&format!(
            "2027-01-15T08:00:{second:02}.{milli:03}Z \x1b[{color}m{level:5}\x1b[0m GET /api/v1/items?page={number} 200 {took} ms\r\n"
        ));
        number += 1;
    }
    output.push_str(last);
    output.into_bytes()
}

/// A full screen of `size` in which each cell has a new character and a new color, and each
/// ten cells a new background: the worst case of the painter.
fn full_screen(frame: usize, size: GridSize) -> Vec<u8> {
    let mut screen = String::from("\x1b[H");
    for row in 0..usize::from(size.rows) {
        screen.push_str(&format!("\x1b[{};1H", row + 1));
        for column in 0..usize::from(size.columns) {
            let index = row * usize::from(size.columns) + column + frame;
            if column % 10 == 0 {
                screen.push_str(&format!("\x1b[48;5;{}m", 232 + index % 24));
            }
            let character = char::from(b'!' + (index % 90) as u8);
            screen.push_str(&format!("\x1b[38;5;{}m{character}", 16 + index % 216));
        }
    }
    screen.push_str("\x1b[0m");
    screen.into_bytes()
}

#[test]
fn a_frame_of_a_terminal_stays_in_the_frame_limit_with_10_mb_of_output_and_with_a_full_redraw() {
    let web = terminal(1, "web-1");
    let view = with_terminals(view(vec![live("ams-prod", &pods())]), std::slice::from_ref(&web));
    let mut window = window_with_no_gpu(view, 2.0);
    window.set_size(egui::vec2(1900.0, 1400.0));
    window.run();
    open_tab(&mut window, &title(&web));
    grow_dock(&mut window, 700.0);
    let size = web.terminal.size();
    assert!(size.columns >= 200 && size.rows >= 60, "{size:?}");
    let mut warm = Step::new("the first frames of a terminal");
    (0..20).for_each(|_| warm.frame(&mut window));

    // A second thread feeds the output in pieces, as a session does; the frames go on.
    let output = log_output(10 * 1024 * 1024, "the last line of the flood\r\n");
    let fed = AtomicBool::new(false);
    let mut flood = Step::new("a terminal with 10 MB of output");
    std::thread::scope(|scope| {
        scope.spawn(|| {
            for piece in output.chunks(16 * 1024) {
                web.terminal.feed(piece);
            }
            fed.store(true, Ordering::Release);
        });
        while !fed.load(Ordering::Acquire) {
            flood.frame(&mut window);
        }
    });
    flood.frame(&mut window);
    flood.check();
    let shown = rows(&window);
    assert!(shown.iter().any(|row| row == "the last line of the flood"), "{:?}", shown.last());

    let mut redraw = Step::new("a full redraw of a terminal");
    for frame in 0..120 {
        web.terminal.feed(&full_screen(frame, size));
        redraw.frame(&mut window);
    }
    redraw.check();
    let first = char::from(b'!' + (119 % 90) as u8);
    assert!(rows(&window).first().is_some_and(|row| row.starts_with(first)), "the last screen");
}
