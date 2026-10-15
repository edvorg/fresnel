//! The log views on the fake core (M9): the tab of a view with the state of each source,
//! its pane with follow, wrap and long lines, its search and filter, its copy and its saves.
//!
//! A test opens a tab with the action of a panel, reads the request that the shell names to
//! the core, and gives the fake core a view of that request with its lines.
//!
//! - `tab`: the tab, its title and its options. `pane`: follow, wrap and long lines.
//! - `find`: the search and the filter. `copy`: the selection and its copy. `saves`: the
//!   saves of a view. `snapshots`: PNG files. `frame_time`: a frame with 200,000 lines.

mod copy;
mod find;
mod frame_time;
mod pane;
mod saves;
mod snapshots;
mod tab;

use std::sync::Arc;

use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use fresnel_core::app::{Command, CoreView};
use fresnel_core::logs::{LineKind, LogLine, LogRequest, LogView, PodsState, SourceState, SourceView};
use fresnel_core::store::ObjectKey;

use crate::data::{held, live, pods};
use crate::fake::{Window, now, until, view, window};

/// The time of a line `seconds` before the time of the view tests, in nanoseconds.
pub(crate) fn at(seconds_before: f64) -> i64 {
    let now = now().duration_since(std::time::UNIX_EPOCH).expect("a time").as_nanos() as i64;
    now - (seconds_before * 1e9) as i64
}

/// A log line of a source.
pub(crate) fn line(seconds_before: f64, source: u32, text: &str) -> LogLine {
    LogLine {
        time: at(seconds_before),
        source,
        text: text.into(),
        spans: None,
        kind: LineKind::Log,
        late: false,
        cut: false,
    }
}

/// A marker of the app.
pub(crate) fn marker(seconds_before: f64, source: u32, text: &str) -> LogLine {
    LogLine { kind: LineKind::Marker, ..line(seconds_before, source, text) }
}

/// A source of the namespace `scene`.
pub(crate) fn source(pod: &str, container: &str, state: SourceState) -> SourceView {
    SourceView { pod: ObjectKey::new("scene", pod), container: container.into(), state, lines: 0, repeated: 0 }
}

/// The view of a request with these sources and lines, the first line with the number 0.
pub(crate) fn log_view(request: &LogRequest, sources: Vec<SourceView>, lines: Vec<LogLine>) -> LogView {
    LogView {
        request: request.clone(),
        pods: PodsState::Following,
        sources: sources.into(),
        lines: fresnel_core::logs::Lines::new(0, lines),
        dropped: 0,
        bytes: 0,
        unstreamed: 0,
        restarts: 0,
    }
}

/// `count` lines of one source: "line 0007 of the log" and so on, one each 10 ms from
/// 07:56:40. A longer list has the same lines first.
pub(crate) fn numbered(count: usize) -> Vec<LogLine> {
    let text = |number: usize| format!("line {number:04} of the log");
    (0..count).map(|number| line(200.0 - number as f64 / 100.0, 0, &text(number))).collect()
}

/// The view of the fake core with these log views in the clusters of their requests.
pub(crate) fn with_logs(mut view: CoreView, logs: Vec<LogView>) -> CoreView {
    for row in &mut view.clusters {
        if let Some(data) = &mut row.data {
            let own = logs.iter().filter(|log| log.request.cluster == row.id).cloned().map(held);
            data.logs = own.collect();
        }
    }
    view
}

/// Gives the fake core these log views.
pub(crate) fn show(window: &mut Window, logs: Vec<LogView>) {
    let view = window.state().view.clone();
    window.state_mut().view = with_logs(view, logs);
    window.run();
}

/// The requests of the last `Command::ShowLogs` of the views.
pub(crate) fn shown(window: &Window) -> Vec<LogRequest> {
    let mut sent = window.state().sent.iter().rev();
    let requests = sent.find_map(|command| match command {
        Command::ShowLogs(requests) => Some(requests.clone()),
        _ => None,
    });
    requests.unwrap_or_default()
}

/// The window with the panel of the pod `name` and the Logs tab of its view. Gives the
/// request that the shell named to the core.
pub(crate) fn opened(name: &str) -> (Window, LogRequest) {
    let mut window = window(view(vec![live("ams-prod", &pods())]));
    let request = open_logs(&mut window, name);
    (window, request)
}

/// Opens the panel of a pod with the filter of the Pods page: the dock with a log tab has
/// text fields too.
pub(crate) fn open_pod(window: &mut Window, name: &str) {
    let started = std::time::Instant::now();
    window.get_by_label("Filter pods").click();
    window.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    window.get_by_label("Filter pods").type_text(name);
    window.run();
    while !crate::kinds::in_panel(window, name) {
        assert!(started.elapsed() < std::time::Duration::from_secs(30), "in time: the panel of {name}");
        let row = window.query_all_by_role_and_label(Role::Label, name).next();
        if let Some(row) = row.filter(|_| window.query_by_label("updating…").is_none()) {
            row.click();
        }
        window.run();
    }
}

/// Opens the panel of the pod `name` and the Logs tab of its view. Gives its request.
pub(crate) fn open_logs(window: &mut Window, name: &str) -> LogRequest {
    open_pod(window, name);
    window.get_by_role_and_label(Role::Button, "Logs").click();
    window.run();
    shown(window).into_iter().next().expect("the request of the view")
}

/// The texts of the rows of the pane on the screen, from the top.
pub(crate) fn rows(window: &Window) -> Vec<String> {
    let mut rows: Vec<(f32, String)> = window
        .query_all_by_role(Role::ListItem)
        .map(|node| (node.rect().top(), node.accesskit_node().label().unwrap_or_default()))
        .collect();
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    rows.into_iter().map(|(_, text)| text).collect()
}

/// Draws frames until the pane shows a row with this text.
pub(crate) fn until_row(window: &mut Window, text: &str) {
    until(window, &format!("the row {text}"), |window| rows(window).iter().any(|row| row.contains(text)));
}

/// The clipboard text that the last frame gave.
pub(crate) fn copied(window: &Window) -> Option<String> {
    window.output().platform_output.commands.iter().find_map(|command| match command {
        egui::OutputCommand::CopyText(text) => Some(text.clone()),
        _ => None,
    })
}

/// The first row of the pane on the screen.
pub(crate) fn first_row(window: &Window) -> egui_kittest::Node<'_> {
    let mut rows: Vec<egui_kittest::Node<'_>> = window.query_all_by_role(Role::ListItem).collect();
    rows.sort_by(|a, b| a.rect().top().total_cmp(&b.rect().top()));
    rows.into_iter().next().expect("a row on the screen")
}

/// The pane: the place of its rows.
pub(crate) fn pane(window: &Window) -> egui::Rect {
    let rows = window.query_all_by_role(Role::ListItem).map(|node| node.rect());
    rows.reduce(|a, b| a.union(b)).expect("a row on the screen")
}

/// The request of a pod view with the same object and new options.
pub(crate) fn with_options(
    request: &LogRequest,
    change: impl FnOnce(&mut fresnel_core::logs::LogOptions),
) -> LogRequest {
    let mut changed = request.clone();
    change(&mut changed.options);
    changed
}

/// The UID of a pod of the scene.
pub(crate) fn uid(name: &str) -> Arc<str> {
    crate::data::pod(name).metadata.uid.as_deref().expect("a UID").into()
}
