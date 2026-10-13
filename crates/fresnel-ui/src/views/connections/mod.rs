//! The Connections view: one card for each selected cluster (plan section 8.14).
//!
//! A card shows the one state of the cluster, the time since its last answer, its
//! credential and each channel. "Retry now" sends a command; the view does no work itself.
//!
//! - `credential`: the lines of the credential. `channels`: the table of the channels.

mod channels;
mod credential;

use std::time::Duration;

use egui::{Align, Button, Frame, Layout, RichText, ScrollArea, Ui};
use fresnel_core::app::{ClusterRow, Command, CoreView, ShellNote, Startup};
use fresnel_core::cluster::LinkState;
use fresnel_core::kubeconfig::AuthKind;
use fresnel_core::net::ClusterStatus;

use crate::format::{self, Clock};
use crate::shell::Scene;
use crate::theme::{self, PAD, Palette};
use crate::widgets::tone::Tone;
use crate::widgets::{self, dot};

/// The age of an answer changes with no change of a state, so the view asks for a
/// repaint each time this period ends while a cluster is connected.
const AGE_REPAINT: Duration = Duration::from_secs(1);

pub(crate) fn show(ui: &mut Ui, scene: &mut Scene<'_>) {
    let palette = theme::palette(ui);
    let view = scene.view;
    ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        Frame::new().inner_margin(PAD).show(ui, |ui| {
            if let Some(note) = &view.shell_problem {
                shell_note(ui, palette, note);
            }
            let selected: Vec<&ClusterRow> = view.clusters.iter().filter(|row| row.selected).collect();
            if selected.is_empty() {
                ui.label(RichText::new(empty_text(view)).color(palette.text_muted));
            }
            for row in selected {
                ui.push_id(row.id.as_str(), |ui| card(ui, palette, row, scene));
                ui.add_space(PAD);
            }
        });
    });
    if view.clusters.iter().any(|row| row.status().is_some()) {
        ui.ctx().request_repaint_after(AGE_REPAINT);
    }
}

/// What the view tells while it has no card.
fn empty_text(view: &CoreView) -> &'static str {
    match view.startup {
        Startup::Waiting | Startup::Environment => "The login shell gives its environment. Then the clusters show.",
        Startup::Kubeconfig => "The kubeconfig files load.",
        Startup::Ready if view.clusters.is_empty() => {
            "The kubeconfig files have no context. The Journal tab lists the files that the app looked for."
        }
        Startup::Ready => "No cluster is selected. Select a cluster in the top bar.",
    }
}

/// The login shell gave no environment. The text of its profile shows here and nowhere else.
fn shell_note(ui: &mut Ui, palette: &Palette, note: &ShellNote) {
    surface(palette).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new("The login shell gave no environment").strong().color(palette.warn));
        ui.label(RichText::new(&note.problem).color(palette.text_muted));
        ui.label("The app looks for a credential plugin in its own PATH and in the fixed folders.");
        if !note.detail.is_empty() {
            quoted(ui, palette, &note.detail);
        }
    });
    ui.add_space(PAD);
}

fn surface(palette: &Palette) -> Frame {
    Frame::new().fill(palette.bg_surface).stroke((1.0, palette.border)).corner_radius(6).inner_margin(10)
}

/// A text that a child process wrote, as it is.
fn quoted(ui: &mut Ui, palette: &Palette, text: &str) {
    Frame::new().fill(palette.bg_sunken).corner_radius(4).inner_margin(6).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(text).font(theme::mono()).color(palette.text_muted));
    });
}

/// The card of one selected cluster.
fn card(ui: &mut Ui, palette: &Palette, row: &ClusterRow, scene: &mut Scene<'_>) {
    surface(palette).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            dot(ui, scene.colors.color(&row.id, palette));
            ui.label(RichText::new(row.id.as_str()).strong());
            ui.label(RichText::new(endpoint_text(row)).color(palette.text_muted));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let retry = ui.add(Button::new("Retry now")).on_hover_text(RETRY_HINT);
                widgets::name(&retry, &format!("Retry now: {}", row.id));
                if retry.clicked() {
                    scene.commands.push(Command::RetryNow(row.id.clone()));
                }
                ui.label(RichText::new(answer_text(row)).color(palette.text_muted));
                let (tone, state) = Tone::of_health(row.health());
                ui.label(RichText::new(state).strong().color(tone.color(palette)));
            });
        });
        ui.add_space(4.0);
        match &row.link {
            LinkState::Off => {
                ui.label(RichText::new("The connect starts.").color(palette.text_muted));
            }
            LinkState::Failed(error) => {
                let text = format!("The connect did not start: {error}");
                ui.label(RichText::new(text).color(palette.error));
            }
            LinkState::On(status) => connected(ui, palette, status, scene.clock),
        }
    });
}

const RETRY_HINT: &str = "Ends each retry delay of the cluster. A failed plugin runs again.";

/// The server and the credential type of the kubeconfig entry.
fn endpoint_text(row: &ClusterRow) -> String {
    let auth = match &row.auth {
        Some(AuthKind::Plugin(command)) => format!("plugin {command}"),
        Some(AuthKind::Certificate) => "client certificate".to_owned(),
        Some(AuthKind::Anonymous) => "no credential".to_owned(),
        None => String::new(),
    };
    match &row.server {
        Ok(server) => format!("{server}  ·  {auth}"),
        Err(_) => "the app cannot use the kubeconfig entry".to_owned(),
    }
}

/// The time since the last answer of the server (design rule 4).
fn answer_text(row: &ClusterRow) -> String {
    match (row.status(), row.last_answer_age()) {
        (Some(_), Some(age)) => format!("last answer {} ago", format::span(age)),
        (Some(_), None) => "no answer yet".to_owned(),
        (None, _) => String::new(),
    }
}

fn connected(ui: &mut Ui, palette: &Palette, status: &ClusterStatus, clock: &Clock) {
    if let Some(credential) = &status.credential {
        credential::show(ui, palette, credential, clock);
    }
    if let Some(until) = status.throttle.until {
        let text = format!("The server throttles the app (HTTP 429) until {}.", clock.time_of_day(until));
        ui.label(RichText::new(text).color(palette.warn));
    }
    channels::show(ui, palette, status, clock);
}
