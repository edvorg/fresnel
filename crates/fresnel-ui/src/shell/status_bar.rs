//! Status bar: the one state of each selected cluster, the count of the forwards of each
//! state, and the counts of the journal.

use egui::scroll_area::ScrollBarVisibility;
use egui::{Align, Button, Layout, RichText, ScrollArea, Ui};
use fresnel_core::app::ClusterRow;
use fresnel_core::forward::ForwardState;

use super::{Scene, Tab};
use crate::format;
use crate::theme::{self, Palette, SMALL};
use crate::widgets::tone::Tone;
use crate::widgets::{self, dot};

pub(super) const HEIGHT: f32 = 24.0;

pub(super) fn show(ui: &mut Ui, scene: &Scene<'_>, tab: &mut Tab) {
    let palette = theme::palette(ui);
    // The counts take their place first, at the right. Many clusters then scroll in the
    // rest of the bar and cannot push the counts out of the window.
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.add_space(4.0);
        if journal_counts(ui, palette, scene).clicked() {
            *tab = Tab::Journal;
        }
        if forward_counts(ui, palette, scene).is_some_and(|counts| counts.clicked()) {
            *tab = Tab::Forwards;
        }
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| cluster_states(ui, palette, scene, tab));
    });
}

/// The state of each selected cluster. The list scrolls when it is wider than its place.
fn cluster_states(ui: &mut Ui, palette: &Palette, scene: &Scene<'_>, tab: &mut Tab) {
    let list = ScrollArea::horizontal().auto_shrink(false).scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden);
    list.show(ui, |ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.add_space(4.0);
        let selected: Vec<&ClusterRow> = scene.view.clusters.iter().filter(|row| row.selected).collect();
        if selected.is_empty() {
            ui.label(small("no cluster is selected").color(palette.text_faint));
        }
        for row in selected {
            let name = scene.view.config.alias(&row.id).unwrap_or(row.id.as_str());
            if cluster_state(ui, palette, row, name).clicked() {
                *tab = Tab::Connections;
            }
            ui.add_space(8.0);
        }
    });
}

fn small(text: impl Into<String>) -> RichText {
    RichText::new(text).size(SMALL)
}

/// The mark, the name or the alias and the state of one cluster. A click opens the
/// Connections tab.
fn cluster_state(ui: &mut Ui, palette: &Palette, row: &ClusterRow, name: &str) -> egui::Response {
    let (tone, state) = Tone::of_health(row.health());
    dot(ui, tone.color(palette));
    let text = format!("{name} {state}");
    let button = ui.add(Button::new(small(text).color(palette.text_muted)).frame(false));
    widgets::name(&button, &format!("{name}: {state}"));
    match row.reason() {
        Some(reason) => button.on_hover_text(reason),
        None => button,
    }
}

/// The counts of errors and warnings from the start of the app. A click opens the journal.
fn journal_counts(ui: &mut Ui, palette: &Palette, scene: &Scene<'_>) -> egui::Response {
    let journal = &scene.view.journal;
    let color = match (journal.errors, journal.warnings) {
        (0, 0) => palette.text_muted,
        (0, _) => palette.warn,
        _ => palette.error,
    };
    let text = format!("{}   {}", format::count(journal.errors, "error"), format::count(journal.warnings, "warning"));
    ui.add(Button::new(small(text).color(color)).frame(false)).on_hover_text("Open the journal")
}

/// The count of the forwards of each state, in the tone of the worst one. A click opens the
/// Forwards tab. Nothing with no forward.
fn forward_counts(ui: &mut Ui, palette: &Palette, scene: &Scene<'_>) -> Option<egui::Response> {
    let rows = &scene.view.forwards;
    if rows.is_empty() {
        return None;
    }
    let tone = |state: &ForwardState| match state {
        ForwardState::Failed(_) => Tone::Error,
        ForwardState::Degraded { .. } | ForwardState::Reconnecting { .. } => Tone::Warn,
        ForwardState::Active | ForwardState::Starting(_) | ForwardState::Stopped => Tone::Muted,
    };
    let worst = rows.iter().map(|row| tone(&row.view.state)).max_by_key(|tone| match tone {
        Tone::Error => 2,
        Tone::Warn => 1,
        _ => 0,
    });
    let color = worst.unwrap_or(Tone::Muted).color(palette);
    let text = crate::views::forwards::summary(rows);
    let button = ui.add(Button::new(small(&text).color(color)).frame(false)).on_hover_text("Open the forwards");
    widgets::name(&button, &text);
    Some(button)
}
