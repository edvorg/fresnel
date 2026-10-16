//! The Forwards tab of the dock (plan section 12.3): each forward with its state, its
//! target pod, its local address, its counts, the age of the data of its target and its last
//! error, and the buttons of the chosen one.

use std::sync::Arc;
use std::time::Duration;

use egui::{Button, OpenUrl, RichText, ScrollArea, Ui};
use fresnel_core::app::{Command, ForwardRow};
use fresnel_core::config::forwards::{forget_edit, save_edits};
use fresnel_core::forward::ForwardState;

use super::texts;
use crate::shell::Scene;
use crate::theme::{self, BAR_HEIGHT, PAD, ROW_HEIGHT};
use crate::widgets;
use crate::widgets::mini_table::{self, Column, Text};

/// The forward that the user chose. `None`: the first one.
#[derive(Debug, Default)]
pub(crate) struct ForwardsTab {
    chosen: Option<u64>,
}

const COLUMNS: [Column; 9] = [
    Column::fixed("State", 200.0),
    Column::fixed("Cluster", 110.0),
    Column::fill("Target"),
    Column::fixed("Local address", 170.0),
    Column::fill("Pod"),
    Column::fixed("Connections", 150.0),
    Column::fixed("Bytes", 120.0),
    Column::fill("Data"),
    Column::fill("Last error"),
];

impl ForwardsTab {
    pub(crate) fn show(&mut self, ui: &mut Ui, scene: &mut Scene<'_>) {
        let palette = theme::palette(ui);
        let rows = Arc::clone(&scene.view.forwards);
        let first = rows.first().map(|row| row.id);
        let chosen = self.chosen.filter(|id| rows.iter().any(|row| row.id == *id)).or(first);
        ui.horizontal(|ui| {
            ui.set_height(BAR_HEIGHT - 4.0);
            ui.add_space(PAD);
            if rows.is_empty() {
                let text = "No forward yet. \"Forward a port\" of a pod, a Service or a workload starts one.";
                ui.label(RichText::new(text).color(palette.text_faint));
                return;
            }
            ui.label(RichText::new(texts::summary(&rows)).color(palette.text_muted));
            if let Some(row) = rows.iter().find(|row| Some(row.id) == chosen) {
                ui.add_space(12.0);
                buttons(ui, scene, row);
            }
        });
        // The time to the next attempt of a reconnect changes each second.
        if rows.iter().any(|row| matches!(row.view.state, ForwardState::Reconnecting { .. })) {
            ui.ctx().request_repaint_after(Duration::from_secs(1));
        }
        if rows.is_empty() {
            return;
        }
        egui::Frame::new().inner_margin(egui::Margin { left: PAD as i8, ..egui::Margin::ZERO }).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.style_mut().interaction.selectable_labels = false;
            mini_table::header(ui, "forwards", &COLUMNS);
            ScrollArea::vertical().auto_shrink(false).show_rows(ui, ROW_HEIGHT, rows.len(), |ui, range| {
                for row in rows.get(range).unwrap_or_default() {
                    let name = format!("Forward of {}", row.view.rule);
                    let chosen_row = Some(row.id) == chosen;
                    let response =
                        mini_table::row_to_choose(ui, row.id, &COLUMNS, &cells(row, scene), (chosen_row, &name));
                    if response.clicked() {
                        self.chosen = Some(row.id);
                    }
                }
            });
        });
    }
}

/// The cells of one forward.
fn cells(row: &ForwardRow, scene: &Scene<'_>) -> [Text; 9] {
    let view = &row.view;
    let (state, tone) = texts::state(view, scene.clock);
    let (pod, pod_tone) = texts::pod(view);
    let (data, data_tone) = texts::data(view, scene.clock);
    let (error, error_tone) = texts::last_error(view, scene.clock);
    let cluster = scene.view.config.alias(&view.rule.cluster).unwrap_or(view.rule.cluster.as_str()).to_owned();
    [
        Text::new(state).state(tone),
        Text::new(cluster),
        Text::new(view.rule.target()),
        Text::new(texts::addresses(view)).mono(),
        Text::new(pod).tone(pod_tone),
        Text::new(texts::connections(view)),
        Text::new(texts::bytes(view)),
        Text::new(data).tone(data_tone),
        Text::new(error).tone(error_tone),
    ]
}

/// The buttons of the chosen forward: copy and open its address, stop or start it, "retry
/// now", save or forget its rule, and remove it.
fn buttons(ui: &mut Ui, scene: &mut Scene<'_>, row: &ForwardRow) {
    let view = &row.view;
    let address = view.address();
    let target = view.rule.to_string();
    let button = |ui: &mut Ui, text: &str, enabled: bool| {
        let response = ui.add_enabled(enabled, Button::new(text).small());
        widgets::name(&response, &format!("{text}: {target}"));
        response.clicked()
    };
    if button(ui, "Copy address", address.is_some())
        && let Some(address) = address
    {
        ui.ctx().copy_text(address.to_string());
    }
    if button(ui, "Open in browser", address.is_some())
        && let Some(address) = address
    {
        ui.ctx().open_url(OpenUrl::new_tab(format!("http://{address}/")));
    }
    let stopped = matches!(view.state, ForwardState::Stopped | ForwardState::Failed(_));
    if stopped {
        if button(ui, "Start", true) {
            scene.commands.push(Command::RestartForward(row.id));
        }
    } else if button(ui, "Stop", true) {
        scene.commands.push(Command::StopForward(row.id));
    }
    let retries = matches!(view.state, ForwardState::Reconnecting { .. } | ForwardState::Degraded(_));
    if button(ui, "Retry now", retries) {
        scene.commands.push(Command::RetryForward(row.id));
    }
    match row.saved {
        true if button(ui, "Forget", true) => {
            scene.commands.push(Command::EditSettings(vec![forget_edit(&view.rule)]));
        }
        false if button(ui, "Save", true) => scene.commands.push(Command::EditSettings(save_edits(&view.rule))),
        _ => {}
    }
    let removed = ui.add_enabled(!row.saved, Button::new("Remove").small());
    widgets::name(&removed, &format!("Remove: {target}"));
    let removed = removed.on_disabled_hover_text("A saved forward stays in the list: Forget it first.");
    if removed.clicked() {
        scene.commands.push(Command::RemoveForward(row.id));
    }
}
