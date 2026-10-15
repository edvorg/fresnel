//! The bar of a log tab: the options of the view, the switches of the pane, the search and
//! the filter, the copy and the saves (plan section 12.7, research 05 section 5.7).
//!
//! An option of the streams (container, previous instance, tail, since) is a new request;
//! the times, the wrap and follow change the pane only.

use std::sync::Arc;
use std::time::Duration;

use egui::containers::menu::{MenuButton, MenuConfig};
use egui::{Button, ComboBox, Key, PopupCloseBehavior, RichText, TextEdit, Ui};
use fresnel_core::app::ClusterData;
use fresnel_core::kinds::Kind;
use fresnel_core::logs::{Containers, LogOptions, LogRequest, LogView, SavePart, SaveState, SaveView, Since};
use k8s_openapi::api::core::v1::Pod;

use super::find::Find;
use crate::format;
use crate::theme::{self, SMALL};
use crate::widgets;
use crate::widgets::tone::Tone;

/// The tails of a view: the last lines of each log at the start.
const TAILS: [(Option<u32>, &str); 4] =
    [(Some(100), "Last 100"), (Some(1000), "Last 1,000"), (Some(10_000), "Last 10,000"), (None, "All lines")];
/// The first times of the lines of a view.
const SINCE: [(Option<u64>, &str); 6] = [
    (None, "Any time"),
    (Some(300), "Last 5 min"),
    (Some(900), "Last 15 min"),
    (Some(3600), "Last hour"),
    (Some(6 * 3600), "Last 6 h"),
    (Some(24 * 3600), "Last 24 h"),
];

/// What the user asked for in the bar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Asked {
    /// New options of the streams: a new view.
    Options(LogOptions),
    /// The next hit, or the hit before with `false`.
    Go(bool),
    Copy,
    Save(SavePart),
    /// Stops the save with this number.
    Cancel(u64),
}

/// The fields of the tab that the bar changes.
pub(super) struct State<'a> {
    pub(super) wrap: &'a mut bool,
    pub(super) times: &'a mut bool,
    pub(super) follow: &'a mut bool,
    pub(super) find: &'a mut Find,
    /// True with a selection to copy.
    pub(super) selected: bool,
}

/// What the bar reads.
pub(super) struct Parts<'a> {
    pub(super) data: Option<&'a ClusterData>,
    pub(super) view: Option<&'a LogView>,
    /// The last save of the view.
    pub(super) save: Option<&'a SaveView>,
}

/// Draws the bar.
pub(super) fn show(ui: &mut Ui, state: &mut State<'_>, request: &LogRequest, parts: &Parts<'_>) -> Vec<Asked> {
    let mut asked = Vec::new();
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        if let Some(options) = options(ui, request, parts) {
            asked.push(Asked::Options(options));
        }
        ui.separator();
        ui.checkbox(state.times, "Times");
        ui.checkbox(state.wrap, "Wrap");
        ui.checkbox(state.follow, "Follow");
    });
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        asked.extend(search(ui, state.find, parts.view));
        let copy = ui.add_enabled(state.selected, Button::new("Copy"));
        let copy = copy.on_disabled_hover_text("Select some text first: a drag, a double click or Cmd+A.");
        if copy.clicked() {
            asked.push(Asked::Copy);
        }
        asked.extend(saves(ui, request, parts));
    });
    asked
}

/// The options of the streams. Gives new ones when the user changed one.
fn options(ui: &mut Ui, request: &LogRequest, parts: &Parts<'_>) -> Option<LogOptions> {
    let mut options = request.options.clone();
    let containers = containers(parts.data, request, parts.view);
    let text = |containers: &Containers| match containers {
        Containers::Default => "Default container".to_owned(),
        Containers::Named(name) => format!("Container {name}"),
        Containers::All => "All containers".to_owned(),
    };
    let menu = ComboBox::from_id_salt("container").selected_text(text(&options.containers)).show_ui(ui, |ui| {
        ui.selectable_value(&mut options.containers, Containers::Default, text(&Containers::Default));
        for (name, _) in &containers {
            let named = Containers::Named(Arc::clone(name));
            ui.selectable_value(&mut options.containers, named.clone(), text(&named));
        }
        ui.selectable_value(&mut options.containers, Containers::All, text(&Containers::All));
    });
    widgets::name(&menu.response, "Containers of the view");
    let restarted = containers.iter().any(|(name, restarted)| *restarted && shows(&options.containers, name));
    let previous =
        ui.add_enabled(restarted || options.previous, egui::Checkbox::new(&mut options.previous, "Previous"));
    previous
        .on_hover_text("The log of the instance of each container before its last restart.")
        .on_disabled_hover_text("No container of the view restarted: it has no earlier instance.");
    let tail = TAILS.iter().find(|(tail, _)| *tail == options.tail).map_or("Tail", |(_, text)| text);
    let menu = ComboBox::from_id_salt("tail").selected_text(tail).show_ui(ui, |ui| {
        for (tail, text) in TAILS {
            ui.selectable_value(&mut options.tail, tail, text);
        }
    });
    widgets::name(&menu.response, "Tail of the view");
    let seconds = match options.since {
        Some(Since::Last(time)) => Some(time.as_secs()),
        _ => None,
    };
    let mut chosen = seconds;
    let since = SINCE.iter().find(|(since, _)| *since == seconds).map_or("Since", |(_, text)| text);
    let menu = ComboBox::from_id_salt("since").selected_text(since).show_ui(ui, |ui| {
        for (since, text) in SINCE {
            ui.selectable_value(&mut chosen, since, text);
        }
    });
    widgets::name(&menu.response, "Since of the view");
    if chosen != seconds {
        options.since = chosen.map(|seconds| Since::Last(Duration::from_secs(seconds)));
    }
    (options != request.options).then_some(options)
}

/// True when the containers of a view have this one.
fn shows(containers: &Containers, name: &str) -> bool {
    match containers {
        Containers::Named(named) => &**named == name,
        Containers::Default | Containers::All => true,
    }
}

/// The containers of the pods of a view in the sequence of kubectl, each with true when it
/// restarted: an earlier instance has a log. A workload view reads the pods of its sources.
fn containers(data: Option<&ClusterData>, request: &LogRequest, view: Option<&LogView>) -> Vec<(Arc<str>, bool)> {
    let Some(data) = data else {
        return Vec::new();
    };
    let mut keys = vec![&request.key];
    if request.kind != Kind::Pod {
        keys = view.map(|view| view.sources.iter().map(|source| &source.pod).collect()).unwrap_or_default();
    }
    let mut found: Vec<(Arc<str>, bool)> = Vec::new();
    for pod in keys.into_iter().filter_map(|key| data.pods.items.get(key)) {
        for (name, restarted) in of_pod(pod) {
            match found.iter_mut().find(|(known, _)| *known == name) {
                Some((_, known)) => *known |= restarted,
                None => found.push((name, restarted)),
            }
        }
    }
    found
}

/// The containers of a pod, the init and the ephemeral ones too.
fn of_pod(pod: &Pod) -> Vec<(Arc<str>, bool)> {
    let Some(spec) = pod.spec.as_ref() else {
        return Vec::new();
    };
    let status = pod.status.as_ref();
    let statuses = status.into_iter().flat_map(|status| {
        [&status.init_container_statuses, &status.container_statuses, &status.ephemeral_container_statuses]
    });
    let statuses: Vec<_> = statuses.flatten().flatten().collect();
    let restarted = |name: &str| {
        statuses.iter().any(|status| {
            status.name == name
                && status.restart_count > 0
                && status.last_state.as_ref().is_some_and(|last| last.terminated.is_some())
        })
    };
    let init = spec.init_containers.iter().flatten().map(|container| container.name.as_str());
    let ephemeral = spec.ephemeral_containers.iter().flatten().map(|container| container.name.as_str());
    let names = init.chain(spec.containers.iter().map(|container| container.name.as_str())).chain(ephemeral);
    names.map(|name| (Arc::from(name), restarted(name))).collect()
}

/// The search with its hits, and the filter. The progress of the worker is in this row: a
/// row of its own above the pane would move the rows of the pane at each key.
fn search(ui: &mut Ui, find: &mut Find, view: Option<&LogView>) -> Vec<Asked> {
    let mut asked = Vec::new();
    let fill = theme::palette(ui).bg_raised;
    let field = TextEdit::singleline(&mut find.search).hint_text("Search").background_color(fill);
    let field = ui.add(field.desired_width(170.0));
    widgets::name(&field, "Search the log");
    if field.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter)) {
        asked.push(Asked::Go(!ui.input(|input| input.modifiers.shift)));
        field.request_focus();
    }
    let any = find.search().is_some();
    for (text, name, forward) in [("‹", "The hit before", false), ("›", "The next hit", true)] {
        let button = ui.add_enabled(any, Button::new(text).small());
        widgets::name(&button, name);
        if button.clicked() {
            asked.push(Asked::Go(forward));
        }
    }
    if let Some(count) = find.count() {
        ui.label(RichText::new(count).size(SMALL).color(theme::palette(ui).text_muted));
    }
    let field = TextEdit::singleline(&mut find.filter).hint_text("Filter").background_color(fill);
    let field = ui.add(field.desired_width(150.0));
    widgets::name(&field, "Filter the log");
    if let Some(progress) = view.and_then(|view| find.progress(&view.lines)) {
        ui.label(RichText::new(progress).size(SMALL).color(theme::palette(ui).text_muted));
    }
    asked
}

/// The menu of the saves, and the state of the last save of the view.
fn saves(ui: &mut Ui, request: &LogRequest, parts: &Parts<'_>) -> Vec<Asked> {
    let mut asked = Vec::new();
    let config = MenuConfig::new().close_behavior(PopupCloseBehavior::CloseOnClickOutside);
    let full =
        if request.options.previous { "Save the full log of the previous instance" } else { "Save the full log" };
    MenuButton::new("Save").config(config).ui(ui, |ui| {
        if ui.button("Save the lines of the view").clicked() {
            asked.push(Asked::Save(SavePart::Buffer));
            ui.close();
        }
        if ui.button(full).clicked() {
            asked.push(Asked::Save(SavePart::Full));
            ui.close();
        }
    });
    let Some(save) = parts.save else {
        return asked;
    };
    let palette = theme::palette(ui);
    let (text, tone) = save_text(save);
    ui.label(RichText::new(text).size(SMALL).color(tone.color(palette)));
    if !save.state.is_final() {
        let cancel = ui.add(Button::new("Cancel").small());
        widgets::name(&cancel, "Cancel the save");
        if cancel.clicked() {
            asked.push(Asked::Cancel(save.number));
        }
    }
    asked
}

/// The state of a save in words.
fn save_text(save: &SaveView) -> (String, Tone) {
    let file = save.file.as_ref().map_or_else(String::new, |file| file.display().to_string());
    let size = |bytes: u64| format!("{}B", format::memory(bytes));
    match &save.state {
        SaveState::Writing { bytes } => (format!("Saving {} to {file}…", size(*bytes)), Tone::Info),
        SaveState::Done { bytes, skipped } if skipped.is_empty() => {
            (format!("Saved {} to {file}", size(*bytes)), Tone::Ok)
        }
        SaveState::Done { bytes, skipped } => {
            let note = skipped.join("; ");
            (format!("Saved {} to {file}; no log of {note}", size(*bytes)), Tone::Warn)
        }
        SaveState::Failed(error) => (format!("The save failed: {error}"), Tone::Error),
        SaveState::Cancelled => ("The save stopped, and its file is gone.".to_owned(), Tone::Muted),
    }
}
