//! PNG files of the log views, for a look by a person: a workload view with the colors of
//! its program, a marker, a search and a selection, the same in the light theme, and the
//! window of a long line.

use std::sync::Arc;

use egui::accesskit::Role;
use egui::{Event, Modifiers, PointerButton};
use egui_kittest::kittest::Queryable as _;
use fresnel_core::CoreError;
use fresnel_core::logs::{Color, LogLine, LogView, SourceState, Span, Style};
use fresnel_core::net::ErrorClass;

use super::{line, log_view, marker, open_logs, rows, show, source, until_row};
use crate::data::{live, pods};
use crate::fake::{SECOND, now, until, view, window_with};
use crate::snapshots::{SCALE, save_clean};

const WEB: &str = "web-6888b778b6-75md7";

/// A line with the colors of a program: a level in a color and a word in bold.
fn colored(seconds: f64, source: u32, level: &str, color: u8, text: &str) -> LogLine {
    let text = format!("{level} {text}");
    let spans = [
        Span { start: 0, style: Style { foreground: Some(Color::Palette(color)), bold: true, ..Style::default() } },
        Span { start: level.len() as u32, style: Style::default() },
    ];
    LogLine { spans: Some(Arc::from(spans)), ..line(seconds, source, &text) }
}

fn scene_view(request: &fresnel_core::logs::LogRequest) -> LogView {
    let reconnecting = SourceState::Reconnecting {
        reason: CoreError::new(ErrorClass::Network, "the connection is silent"),
        at: now() + 4 * SECOND,
    };
    let sources = vec![
        source("web-6888b778b6-75md7", "web", SourceState::Streaming),
        source("web-6888b778b6-p2twl", "web", reconnecting),
        source(
            "web-6888b778b6-p2twl",
            "proxy",
            SourceState::Waiting { reason: "the container did not start: CrashLoopBackOff".into() },
        ),
    ];
    let mut lines = Vec::new();
    for second in 0..40 {
        let at = 60.0 - f64::from(second);
        let source = (second % 2) as u32;
        lines.push(match second % 7 {
            3 => colored(at, source, "WARN", 3, "the cache is cold, the request took 840 ms"),
            5 => colored(at, source, "ERROR", 1, "upstream api.shop.svc refused the connection"),
            _ => colored(at, source, "INFO", 2, &format!("GET /api/v1/items?page={second} 200 12ms")),
        });
        if second == 20 {
            lines.push(marker(
                at,
                1,
                "the stream broke; a new stream started at 07:59:40 and dropped 3 lines that came again",
            ));
        }
    }
    lines.push(line(1.0, 0, &format!("{{\"request\": \"{}\"}}", "x".repeat(4000))));
    log_view(request, sources, lines)
}

#[test]
fn the_log_views_in_pictures() {
    for (theme, name) in [(egui::Theme::Dark, "logs-workload"), (egui::Theme::Light, "logs-workload-light")] {
        let mut window = window_with(view(vec![live("ams-prod", &pods())]), theme, SCALE);
        let request = open_logs(&mut window, WEB);
        show(&mut window, vec![scene_view(&request)]);
        until_row(&mut window, "INFO GET /api/v1/items?page=39");
        window.get_by_label("Search the log").click();
        window.get_by_label("Search the log").type_text("refused");
        until(&mut window, "the hits", |window| window.query_by_label_contains("lines").is_some());
        window.get_by_label("The next hit").click();
        window.run();
        // A selection over two rows.
        let mut places: Vec<egui::Rect> = window.query_all_by_role(Role::ListItem).map(|node| node.rect()).collect();
        places.sort_by(|a, b| a.top().total_cmp(&b.top()));
        if let (Some(first), Some(second)) = (places.get(2), places.get(3)) {
            let (start, end) = (first.center(), second.center());
            let button = |pos, pressed| Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            for event in [Event::PointerMoved(start), button(start, true), Event::PointerMoved(end), button(end, false)]
            {
                window.event(event);
                window.run();
            }
        }
        save_clean(&mut window, name);
        assert!(!rows(&window).is_empty());
    }

    let mut window = window_with(view(vec![live("ams-prod", &pods())]), egui::Theme::Dark, SCALE);
    let request = open_logs(&mut window, WEB);
    show(&mut window, vec![scene_view(&request)]);
    until(&mut window, "the button of the long line", |window| {
        window.query_by_label_contains("Show the full line").is_some()
    });
    window.query_all_by_label_contains("Show the full line").next().expect("the button").click();
    window.run();
    save_clean(&mut window, "logs-full-line");
}
