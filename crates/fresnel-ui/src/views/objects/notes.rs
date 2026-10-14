//! The note of each selected cluster above the table of a kind: the state of the store of
//! the kind with the reason, and how old its rows are (design rules 4 and 5).
//!
//! A cluster whose store is live has no note. Each other cluster has a line of its own,
//! so two clusters that fail show two reasons (weakness 3 of plan section 6), and a list
//! with no row never stands for an error.

use std::time::SystemTime;

use egui::{Label, RichText, ScrollArea, Ui};
use fresnel_core::app::ClusterRow;
use fresnel_core::cluster::{ClusterId, LinkState};
use fresnel_core::store::{Snapshot, StoreState, Wait};

use super::Nouns;
use crate::format::{self, Clock};
use crate::shell::Scene;
use crate::theme::{Palette, ROW_HEIGHT};
use crate::widgets::dot;
use crate::widgets::table::cells::ring;
use crate::widgets::tone::Tone;

/// The most notes that show with no scroll. More clusters than this scroll in their place
/// and leave the table its place.
const LINES: f32 = 4.0;

/// Since when the rows of a store are old, for the mark of each row. `None` for a live
/// store: its rows are current.
pub(crate) fn old_since<T>(snapshot: &Snapshot<T>, clock: &Clock) -> Option<String> {
    (!snapshot.state.is_live()).then(|| old_text(snapshot.last_contact, clock))
}

/// The mark of an object of a store that is not live, from the last data of the store:
/// the same words in a row, in the panel and in a hit of the quick search.
pub(crate) fn old_text(last_contact: Option<SystemTime>, clock: &Clock) -> String {
    match last_contact {
        Some(time) => format!("old since {}", clock.time_of_day(time)),
        None => "old: the server gave no answer yet".to_owned(),
    }
}

/// What a note reads of the store of the kind of one cluster.
pub(super) struct Facts<'a> {
    state: &'a StoreState,
    last_contact: Option<SystemTime>,
    listed: bool,
    empty: bool,
}

impl<'a> Facts<'a> {
    pub(super) fn of<T>(snapshot: &'a Snapshot<T>) -> Self {
        let (state, last_contact, listed) = (&snapshot.state, snapshot.last_contact, snapshot.listed);
        Self { state, last_contact, listed, empty: snapshot.items.is_empty() }
    }
}

/// The tone and the text of the note of one selected cluster. `facts` is `None` for a
/// connected cluster whose store of the full objects of the kind did not start yet.
fn note(row: &ClusterRow, facts: Option<Facts<'_>>, nouns: &Nouns, clock: &Clock) -> Option<(Tone, String)> {
    if row.data.is_none() {
        return Some(match &row.link {
            LinkState::Failed(error) => (Tone::Error, format!("the connect did not start: {error}")),
            LinkState::Off | LinkState::On(_) => (Tone::Info, "the connect starts".to_owned()),
        });
    }
    let Some(store) = facts else {
        let text = format!("{}: the store of the full objects starts. No {} is loaded.", nouns.title, nouns.one);
        return Some((Tone::Info, text));
    };
    let state = store.state.to_string();
    let (tone, state) = match store.state {
        StoreState::Live => return None,
        // The store waits for the cluster, so the note has the tone and the reason of
        // the cluster.
        StoreState::Waiting(Wait::Channel | Wait::Credential) => {
            let reason = row.reason().map_or_else(String::new, |reason| format!(": {reason}"));
            (Tone::of_health(row.health()).0, format!("{state}{reason}"))
        }
        StoreState::Waiting(Wait::ListSlot | Wait::Stream) | StoreState::Listing { .. } => (Tone::Info, state),
        StoreState::Attempt { .. } => (Tone::Warn, state),
        StoreState::Backoff { until, .. } => {
            (Tone::Warn, format!("{state}; next attempt in {}", format::span(clock.until(*until))))
        }
        StoreState::Throttled { until } => (Tone::Warn, format!("{state} until {}", clock.time_of_day(*until))),
        StoreState::Forbidden { .. } | StoreState::NotSupported { .. } | StoreState::InternalError { .. } => {
            (Tone::Error, state)
        }
        StoreState::Stopped => (Tone::Muted, state),
    };
    let rows = match store.last_contact {
        Some(time) if store.listed => format!("Its rows are old since {}.", clock.time_of_day(time)),
        Some(_) => "Its first list is not complete.".to_owned(),
        None => format!("No {} is loaded.", nouns.one),
    };
    Some((tone, format!("{}: {state}. {rows}", nouns.title)))
}

/// Draws the note of each selected cluster that has one. `facts` gives the store of the
/// kind of a connected cluster.
pub(super) fn show<'a>(
    ui: &mut Ui,
    palette: &Palette,
    scene: &Scene<'_>,
    nouns: &Nouns,
    facts: impl Fn(&ClusterId) -> Option<Facts<'a>>,
) {
    let selected = scene.view.clusters.iter().filter(|row| row.selected);
    let notes: Vec<(&ClusterRow, Tone, String, bool)> = selected
        .filter_map(|row| {
            let facts = facts(&row.id);
            let has_rows = facts.as_ref().is_some_and(|facts| !facts.empty);
            note(row, facts, nouns, scene.clock).map(|(tone, text)| (row, tone, text, has_rows))
        })
        .collect();
    if notes.is_empty() {
        return;
    }
    let list = ScrollArea::vertical().id_salt("notes").max_height(LINES * ROW_HEIGHT).auto_shrink([false, true]);
    list.show(ui, |ui| {
        for (row, tone, text, has_rows) in &notes {
            ui.horizontal(|ui| {
                ui.set_height(ROW_HEIGHT - 2.0);
                ui.add_space(12.0);
                ui.spacing_mut().item_spacing.x = 6.0;
                dot(ui, scene.colors.color(&row.id, palette));
                ui.label(RichText::new(row.id.as_str()).strong());
                // The mark of the rows of this cluster, so that a person knows it again.
                if *has_rows {
                    ring(ui, palette);
                }
                ui.add(Label::new(RichText::new(text).color(tone.color(palette))).truncate());
            });
        }
    });
    let line = ui.available_rect_before_wrap();
    ui.painter().hline(line.x_range(), line.top(), (1.0, palette.border));
}
