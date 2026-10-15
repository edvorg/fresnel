//! The search and the filter of a log view: the hits, the next and the one before, and the
//! lines that the filter keeps with each marker.

use egui::Key;
use egui_kittest::kittest::Queryable as _;
use fresnel_core::logs::{LogLine, SourceState};

use super::{line, log_view, marker, opened, rows, show, source, until_row};
use crate::fake::{shows, until};

const WEB: &str = "web-6888b778b6-75md7";

/// 1,000 lines; each hundredth one has an error.
fn errors() -> Vec<LogLine> {
    let text = |number: usize| {
        if number.is_multiple_of(100) {
            format!("{number:04} ERROR the cache missed")
        } else {
            format!("{number:04} ok")
        }
    };
    (0..1000).map(|number| line((1000 - number) as f64 / 100.0, 0, &text(number))).collect()
}

#[test]
fn the_search_counts_its_hits_and_goes_to_the_next_and_the_one_before() {
    let (mut window, request) = opened(WEB);
    show(&mut window, vec![log_view(&request, vec![source(WEB, "web", SourceState::Streaming)], errors())]);
    until_row(&mut window, "0999 ok");
    window.get_by_label("Search the log").click();
    window.get_by_label("Search the log").type_text("error");
    until(&mut window, "the hits", |window| shows(window, "10 lines"));

    window.get_by_label("The next hit").click();
    window.run();
    assert!(shows(&window, "1 of 10 lines"), "after the last line comes the first hit");
    until_row(&mut window, "0000 ERROR the cache missed");
    window.get_by_label("The next hit").click();
    window.run();
    assert!(shows(&window, "2 of 10 lines"));
    until_row(&mut window, "0100 ERROR");
    window.get_by_label("The hit before").click();
    window.get_by_label("The hit before").click();
    window.run();
    assert!(shows(&window, "10 of 10 lines"), "before the first hit comes the last one");
    until_row(&mut window, "0900 ERROR");

    // Enter in the field goes to the next hit; a capital letter keeps the case.
    window.get_by_label("Search the log").click();
    window.key_press(Key::Enter);
    window.run();
    assert!(shows(&window, "1 of 10 lines"));
    window.get_by_label("Search the log").type_text("X");
    until(&mut window, "no hit", |window| shows(window, "no line"));
}

#[test]
fn the_filter_shows_the_lines_with_its_text_and_each_marker_and_the_search_reads_them() {
    let (mut window, request) = opened(WEB);
    let lines = vec![
        line(5.0, 0, "GET /a 200"),
        line(4.0, 0, "POST /b 500"),
        marker(3.0, 0, "the stream started again"),
        line(2.0, 0, "GET /c 404"),
        line(1.0, 0, "tick"),
    ];
    show(&mut window, vec![log_view(&request, vec![source(WEB, "web", SourceState::Streaming)], lines)]);
    until_row(&mut window, "tick");
    window.get_by_label("Filter the log").click();
    window.get_by_label("Filter the log").type_text("GET");
    until(&mut window, "the kept lines", |window| rows(window).len() == 3);
    let texts: Vec<String> = rows(&window)
        .iter()
        .map(|row| row.split_once(' ').map(|(_, text)| text.to_owned()).unwrap_or_default())
        .collect();
    assert_eq!(texts, ["GET /a 200", "the stream started again", "GET /c 404"]);
    window.get_by_label("Search the log").click();
    window.get_by_label("Search the log").type_text("404");
    until(&mut window, "the hit among the kept lines", |window| shows(window, "1 line"));
}
