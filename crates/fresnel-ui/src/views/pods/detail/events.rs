//! The Events of the pod of the detail panel, the newest first.
//!
//! The Event index tells what its answer is good for. The section shows that next to the
//! Events, so "no Events" and "the Events are not loaded" never look the same.

use std::time::SystemTime;

use egui::{Align, Label, Layout, RichText, Ui};
use fresnel_core::app::ClusterData;
use fresnel_core::index::{Basis, IndexState, last_seen};
use fresnel_core::kinds::Kind;
use k8s_openapi::api::core::v1::Event;

use super::{NONE, heading};
use crate::format::{self, Clock};
use crate::theme::{Palette, SMALL};
use crate::widgets::dot;
use crate::widgets::table::model::ObjectId;
use crate::widgets::tone::Tone;

/// The most Events that the section draws. An object can have hundreds of them.
const LIMIT: usize = 100;

/// The pod of the panel and the data of its cluster.
pub(super) struct Source<'a> {
    pub(super) id: &'a ObjectId,
    /// `None` for a cluster that is not connected.
    pub(super) data: Option<&'a ClusterData>,
}

pub(super) fn show(ui: &mut Ui, palette: &Palette, clock: &Clock, source: &Source<'_>) {
    heading(ui, palette, "EVENTS");
    let note = |ui: &mut Ui, tone: Tone, text: &str| ui.label(RichText::new(text).color(tone.color(palette)));
    let Some(data) = source.data else {
        note(ui, Tone::Warn, "The cluster is not connected, so the panel has no Events.");
        return;
    };
    let Some(index) = &data.events else {
        note(ui, Tone::Muted, "The settings give the Events of this cluster no store.");
        return;
    };
    let answer = index.events_of(Kind::Pod, &source.id.key, &source.id.uid);
    if let Some(text) = basis_note(&answer.basis) {
        note(ui, Tone::Warn, &text);
    }
    if answer.found.is_empty() {
        if answer.basis.is_loaded() {
            note(ui, Tone::Faint, "No Events.");
        }
        return;
    }
    for event in answer.found.iter().take(LIMIT) {
        entry(ui, palette, clock, event);
    }
    if let Some(more) = answer.found.len().checked_sub(LIMIT).filter(|more| *more > 0) {
        ui.add_space(4.0);
        note(ui, Tone::Muted, &format!("{} do not show.", format::count(more as u64, "older Event")));
    }
}

/// What the Events of the section are good for, when they are not the state of now.
fn basis_note(basis: &Basis) -> Option<String> {
    match basis {
        Basis::Current => None,
        Basis::Old(source) => Some(format!("The Events can be old: their store {}.", source.state)),
        Basis::NotLoaded(source) => Some(format!("The Events are not loaded: their store {}.", source.state)),
        Basis::NotFollowed(IndexState::Failed { reason }) => {
            Some(format!("The Events can be old: the Event index failed ({reason}). It starts again."))
        }
        Basis::NotFollowed(IndexState::Starting) => {
            Some("The Events are not loaded: the Event index starts.".to_owned())
        }
        Basis::NotFollowed(IndexState::Following | IndexState::Stopped) => {
            Some("The Events can be old: the Event index stopped.".to_owned())
        }
    }
}

/// One Event in two lines: the panel is narrow, and the message is the long part.
fn entry(ui: &mut Ui, palette: &Palette, clock: &Clock, event: &Event) {
    let kind = event.type_.as_deref().unwrap_or(NONE);
    let tone = if kind == "Warning" { Tone::Warn } else { Tone::Muted };
    // A source that repeats an Event counts in its series or in its count.
    let count = event.series.as_ref().and_then(|series| series.count).or(event.count).unwrap_or(1);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        dot(ui, tone.color(palette));
        ui.label(RichText::new(event.reason.as_deref().unwrap_or(NONE)).strong());
        ui.label(RichText::new(kind).size(SMALL).color(tone.color(palette)));
        if count > 1 {
            ui.label(RichText::new(format!("×{count}")).size(SMALL).color(palette.text_muted));
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let seen = last_seen(event).map(SystemTime::from);
            let age = seen.map_or_else(|| NONE.to_owned(), |time| format!("{} ago", format::span(clock.since(time))));
            ui.label(RichText::new(age).size(SMALL).color(palette.text_faint));
        });
    });
    let message = event.message.as_deref().unwrap_or_default();
    ui.add(Label::new(RichText::new(message).color(palette.text_muted)).wrap());
    ui.add_space(6.0);
}
