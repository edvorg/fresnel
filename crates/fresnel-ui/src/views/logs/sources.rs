//! The state of each source of a view in the header of its tab (design rule 1), and the
//! notes of the view: how far it knows its pods, the lines that left it at its limit, the
//! containers with no stream, and the restarts of its task.

use egui::{RichText, Ui};
use fresnel_core::app::CoreView;
use fresnel_core::logs::{LogRequest, LogView, PodsState, SourceState, SourceView};

use crate::format::{self, Clock, short};
use crate::theme::{self, SMALL};
use crate::widgets::dot;
use crate::widgets::tone::Tone;

/// The most sources with a line of their own. A workload can have 50; the rest are in a menu.
const LISTED: usize = 6;

/// The tone of the state of one source.
fn tone_of(state: &SourceState) -> Tone {
    match state {
        SourceState::Streaming => Tone::Ok,
        SourceState::Connecting => Tone::Info,
        SourceState::Waiting { .. } | SourceState::Reconnecting { .. } => Tone::Warn,
        SourceState::Ended { .. } => Tone::Muted,
        SourceState::Failed { .. } => Tone::Error,
    }
}

/// The tone of a view in the title of its tab: the worst state of its sources.
pub(super) fn tone(sources: &[SourceView]) -> Tone {
    let rank = |tone: Tone| match tone {
        Tone::Error => 4,
        Tone::Warn => 3,
        Tone::Ok => 1,
        Tone::Info => 2,
        _ => 0,
    };
    sources.iter().map(|source| tone_of(&source.state)).max_by_key(|tone| rank(*tone)).unwrap_or(Tone::Faint)
}

/// "streaming", "reconnecting at 08:00:05 after: …": the state of a source in words.
fn words(state: &SourceState, clock: &Clock) -> String {
    match state {
        SourceState::Reconnecting { reason, at } => {
            format!("reconnecting at {} after: {reason}", clock.time_of_day(*at))
        }
        other => other.to_string(),
    }
}

/// Draws the state of each source and the notes of the view.
pub(super) fn show(ui: &mut Ui, view: &LogView, clock: &Clock) {
    let palette = theme::palette(ui);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        for source in view.sources.iter().take(LISTED) {
            line(ui, source, clock);
        }
        let rest = view.sources.len().saturating_sub(LISTED);
        if rest > 0 {
            ui.menu_button(format!("and {rest} more"), |ui| {
                for source in view.sources.iter().skip(LISTED) {
                    line(ui, source, clock);
                }
            });
        }
    });
    for (tone, text) in notes(view) {
        ui.label(RichText::new(text).size(SMALL).color(tone.color(palette)));
    }
}

/// One source: its state as a dot, its pod and container, and its state in words. The
/// parts are items of the row of the caller, so the row wraps between them; a long reason
/// shows its start, and the pointer on it the full text.
fn line(ui: &mut Ui, source: &SourceView, clock: &Clock) {
    let palette = theme::palette(ui);
    let tone = tone_of(&source.state);
    dot(ui, tone.color(palette));
    let name = format!("{}/{}", source.pod.name, source.container);
    ui.label(RichText::new(short(&name, 48)).size(SMALL));
    let state = words(&source.state, clock);
    let shown = ui.label(RichText::new(short(&state, 72)).size(SMALL).color(tone.color(palette)));
    if state.chars().count() > 72 {
        shown.on_hover_text(state);
    }
    if source.lines > 0 {
        let lines = format::count(source.lines, "line");
        ui.label(RichText::new(lines).size(SMALL).color(palette.text_faint));
    }
}

/// The notes of the view, each one with its tone.
fn notes(view: &LogView) -> Vec<(Tone, String)> {
    let mut notes = Vec::new();
    match view.pods {
        PodsState::Loading => notes.push((Tone::Muted, "The pods are not loaded yet.".to_owned())),
        PodsState::Gone => notes.push((Tone::Warn, "The pod is gone. The view keeps what it read.".to_owned())),
        PodsState::Following if view.sources.is_empty() => {
            notes.push((Tone::Muted, "No pod of the view has a container to read now.".to_owned()));
        }
        PodsState::Following => {}
    }
    if view.dropped > 0 {
        let text = format!("{} older lines left the view at its limit.", format::thousands(view.dropped));
        notes.push((Tone::Warn, text));
    }
    if view.unstreamed > 0 {
        let count = format::count(view.unstreamed as u64, "container");
        notes.push((Tone::Warn, format!("{count} of the view have no stream: the view has its most streams.")));
    }
    if view.restarts > 0 {
        let times = format::count(u64::from(view.restarts), "time");
        let text =
            format!("The view started again {times} after an internal error of the app; earlier lines are gone.");
        notes.push((Tone::Error, text));
    }
    notes
}

/// Why a tab has no view: its cluster is not connected, or the core did not start it yet.
pub(super) fn absent(view: &CoreView, request: &LogRequest) -> String {
    let row = view.clusters.iter().find(|row| row.id == request.cluster);
    match row {
        Some(row) if row.data.is_some() => "The view starts.".to_owned(),
        Some(row) => {
            let reason = row.reason().map_or_else(String::new, |reason| format!(": {reason}"));
            format!("{} is not connected{reason}. The view starts when it connects.", request.cluster)
        }
        None => format!("No kubeconfig file has the cluster {} now.", request.cluster),
    }
}
