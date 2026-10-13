//! Status bar: the one state of each selected cluster, and the counts of the journal.

use egui::{Align, Button, Layout, RichText, Ui};
use fresnel_core::app::ClusterRow;

use super::{Scene, Tab};
use crate::format;
use crate::theme::{self, Palette, SMALL};
use crate::widgets::tone::Tone;
use crate::widgets::{self, dot};

pub(super) const HEIGHT: f32 = 24.0;

pub(super) fn show(ui: &mut Ui, scene: &Scene<'_>, tab: &mut Tab) {
    let palette = theme::palette(ui);
    ui.horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.add_space(4.0);
        let selected: Vec<&ClusterRow> = scene.view.clusters.iter().filter(|row| row.selected).collect();
        if selected.is_empty() {
            ui.label(small("no cluster is selected").color(palette.text_faint));
        }
        for row in selected {
            if cluster_state(ui, palette, row).clicked() {
                *tab = Tab::Connections;
            }
            ui.add_space(8.0);
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add_space(4.0);
            if journal_counts(ui, palette, scene).clicked() {
                *tab = Tab::Journal;
            }
        });
    });
}

fn small(text: impl Into<String>) -> RichText {
    RichText::new(text).size(SMALL)
}

/// The mark, the name and the state of one cluster. A click opens the Connections tab.
fn cluster_state(ui: &mut Ui, palette: &Palette, row: &ClusterRow) -> egui::Response {
    let (tone, state) = Tone::of_health(row.health());
    dot(ui, tone.color(palette));
    let text = format!("{} {state}", row.id);
    let button = ui.add(Button::new(small(text).color(palette.text_muted)).frame(false));
    widgets::name(&button, &format!("{}: {state}", row.id));
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
