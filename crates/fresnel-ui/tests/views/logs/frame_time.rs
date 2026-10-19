//! The time of a frame of a log view with 200,000 lines (plan sections 12.7 and 15, M9): a
//! frame takes 16 ms or less while the pane scrolls and wraps, and while the worker thread
//! reads each line for a search, a filter and a copy. A search never holds a frame.
//!
//! The limit holds in a release build: `cargo test --release --package fresnel-ui --test
//! views -- frame_time --nocapture`. A debug build runs the same steps and checks no time.

use egui::{Event, Key, Modifiers};
use egui_kittest::kittest::Queryable as _;
use fresnel_core::logs::{Color, LineKind, LogLine, SourceState, Span, Style};

use super::{at, log_view, open_logs, rows, show, source};
use crate::data::{live, pods};
use crate::fake::{shows, view, window_with_no_gpu};
use crate::frame_time::{Step, wheel};

const LINES: usize = 200_000;
const WEB: &str = "web-6888b778b6-75md7";

/// 200,000 lines of three sources: a word in color in each line, a marker in each
/// 10,000th line, a long line in each other 5,000th line and an error in each other
/// 1,000th line: 160 errors.
fn lines() -> Vec<LogLine> {
    let red = Style { foreground: Some(Color::Palette(1)), bold: true, ..Style::default() };
    let spans: std::sync::Arc<[Span]> =
        std::sync::Arc::from([Span { start: 0, style: Style::default() }, Span { start: 7, style: red }]);
    (0..LINES)
        .map(|number| {
            let text = match number {
                n if n % 10_000 == 0 => "the stream started again".to_owned(),
                n if n % 5_000 == 0 => format!("{n:06} {}", r#"{"key": "value"} "#.repeat(400)),
                n if n % 1_000 == 0 => format!("{n:06} ERROR the request failed with status 503"),
                n => format!("{n:06} GET /api/v1/items?page={n} 200 12ms"),
            };
            let kind = if number % 10_000 == 0 { LineKind::Marker } else { LineKind::Log };
            LogLine {
                time: at((LINES - number) as f64 / 1000.0),
                source: (number % 3) as u32,
                text: text.into(),
                spans: (kind == LineKind::Log).then(|| std::sync::Arc::clone(&spans)),
                kind,
                late: false,
                cut: false,
            }
        })
        .collect()
}

fn key(key: Key, modifiers: Modifiers) -> Event {
    Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
}

#[test]
fn a_frame_with_200_000_lines_stays_in_the_frame_limit_during_scroll_search_filter_wrap_and_copy() {
    let mut window = window_with_no_gpu(view(vec![live("ams-prod", &pods())]), 2.0);
    let request = open_logs(&mut window, WEB);
    let sources = ["web-a", "web-b", "web-c"].map(|pod| source(pod, "web", SourceState::Streaming)).to_vec();
    show(&mut window, vec![log_view(&request, sources, lines())]);
    let mut warm = Step::new("logs, the first frames");
    warm.until(&mut window, "the last line", |window| rows(window).iter().any(|row| row.contains("199999 GET")));
    (0..20).for_each(|_| warm.frame(&mut window));

    let mut scroll = Step::new("logs, scroll");
    let pane = super::pane(&window).center();
    scroll.events(&mut window, [Event::PointerMoved(pane)]);
    scroll.events(&mut window, (0..300).map(|_| wheel(90.0)));
    (0..30).for_each(|_| scroll.frame(&mut window));
    assert!(!rows(&window).iter().any(|row| row.contains("199999 GET")), "the pane did scroll");
    scroll.check();

    // The search reads 200,000 lines on its worker thread, one part after the other.
    let mut search = Step::new("logs, search");
    let field = window.get_by_label("Search the log").rect().center();
    search.click(&mut window, field);
    for character in "error".chars() {
        search.events(&mut window, [Event::Text(character.to_string())]);
    }
    search.until(&mut window, "each hit", |window| shows(window, "160 hits"));
    let next = window.get_by_label("The next hit").rect().center();
    search.click(&mut window, next);
    (0..10).for_each(|_| search.frame(&mut window));
    search.check();

    let mut filter = Step::new("logs, filter");
    let field = window.get_by_label("Filter the log").rect().center();
    filter.click(&mut window, field);
    filter.events(&mut window, [Event::Text("503".to_owned())]);
    filter.until(&mut window, "the kept lines", |window| {
        rows(window).iter().all(|row| row.contains("503") || row.contains("started again"))
    });
    filter.check();

    let mut wrap = Step::new("logs, wrap");
    let checkbox = window.get_by_label("Wrap").rect().center();
    wrap.click(&mut window, checkbox);
    let field = window.get_by_label("Filter the log").rect().center();
    wrap.click(&mut window, field);
    wrap.events(&mut window, (0..3).map(|_| key(Key::Backspace, Modifiers::NONE)));
    wrap.events(&mut window, [Event::PointerMoved(pane)]);
    wrap.events(&mut window, (0..200).map(|_| wheel(-120.0)));
    wrap.check();

    // A click on the time of the first row gives the pane the focus; the middle of the pane
    // can be the button of a long line.
    let mut copy = Step::new("logs, select all and copy");
    let time = super::first_row(&window).rect().left_center() + egui::vec2(4.0, 0.0);
    copy.click(&mut window, time);
    copy.events(&mut window, [key(Key::A, Modifiers::COMMAND)]);
    copy.events(&mut window, [Event::Copy]);
    copy.until(&mut window, "the copy", |window| window.query_by_label_contains("Copied 200,000 lines").is_some());
    copy.check();
}
