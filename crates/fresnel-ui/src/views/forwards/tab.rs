//! The Forwards tab of the dock (plan section 12.3): a list with one short line of each
//! forward (its state, its cluster, its target, its first local address and its pod), and the
//! chosen forward at the right with its full state, its counts and its buttons.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use egui::{Color32, FontId, Frame, Margin, Panel, RichText, ScrollArea, TextStyle, Ui};
use fresnel_core::app::ForwardRow;
use fresnel_core::forward::ForwardState;

use super::{chosen, texts};
use crate::shell::Scene;
use crate::theme::{self, BAR_HEIGHT, PAD, ROW_HEIGHT, SMALL};
use crate::widgets::mini_table::{self, Column, Text};
use crate::widgets::tone::Tone;

/// The most width of the State and of the Cluster column. Each one is as wide as its widest
/// text in the list, so the target and the pod get the room that short words leave.
const STATE_MOST: f32 = 190.0;
const CLUSTER_MOST: f32 = 120.0;
/// The room of the dot of a state before its words: the dot and the space after it.
const DOT: f32 = 13.0;
/// The most width of the first local address, as wide as "127.0.0.1:65535" in the mono
/// font. An address of an other bind is longer: the panel of the chosen forward has it.
const ADDRESS_MOST: f32 = 140.0;
/// The width of the part with the chosen forward.
const DETAIL_WIDTH: f32 = 380.0;

/// The forward that the user chose. `None`: the first one.
#[derive(Debug, Default)]
pub(crate) struct ForwardsTab {
    chosen: Option<u64>,
}

impl ForwardsTab {
    pub(crate) fn show(&mut self, ui: &mut Ui, scene: &mut Scene<'_>) {
        let palette = theme::palette(ui);
        let rows = Arc::clone(&scene.view.forwards);
        ui.horizontal(|ui| {
            ui.set_height(BAR_HEIGHT - 4.0);
            ui.add_space(PAD);
            if rows.is_empty() {
                let text = "No forward yet. \"Forward a port\" of a pod, a Service or a workload starts one.";
                ui.label(RichText::new(text).color(palette.text_faint));
            } else {
                ui.label(RichText::new(texts::summary(&rows)).color(palette.text_muted));
            }
        });
        if rows.is_empty() {
            return;
        }
        // The time to the next attempt of a reconnect changes each second.
        if rows.iter().any(|row| matches!(row.view.state, ForwardState::Reconnecting { .. })) {
            ui.ctx().request_repaint_after(Duration::from_secs(1));
        }
        let first = rows.first().map(|row| row.id);
        let chosen = self.chosen.filter(|id| rows.iter().any(|row| row.id == *id)).or(first);
        let detail = Panel::right("chosen forward").resizable(true).default_size(DETAIL_WIDTH).frame(Frame::new());
        detail.show(ui, |ui| {
            if let Some(row) = rows.iter().find(|row| Some(row.id) == chosen) {
                Frame::new().inner_margin(Margin::symmetric(PAD as i8, 4)).show(ui, |ui| chosen::show(ui, scene, row));
            }
        });
        Frame::new().inner_margin(Margin { left: PAD as i8, ..Margin::ZERO }).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.style_mut().interaction.selectable_labels = false;
            let lines: Vec<[Text; 5]> = rows.iter().map(|row| cells(row, scene)).collect();
            let columns = columns(ui, &lines);
            mini_table::header(ui, "forwards", &columns);
            ScrollArea::vertical().auto_shrink(false).show_rows(ui, ROW_HEIGHT, rows.len(), |ui, range| {
                for (row, cells) in rows.iter().zip(&lines).skip(range.start).take(range.len()) {
                    let name = format!("Forward of {}", row.view.rule);
                    let chosen_row = Some(row.id) == chosen;
                    let response = mini_table::row_to_choose(ui, row.id, &columns, cells, (chosen_row, &name));
                    if response.clicked() {
                        self.chosen = Some(row.id);
                    }
                }
            });
        });
    }
}

/// The columns of the list for these lines. A title fits too.
fn columns(ui: &Ui, lines: &[[Text; 5]]) -> [Column; 5] {
    let (body, small) = (TextStyle::Body.resolve(ui.style()), FontId::proportional(SMALL));
    let width = |text: &str, font: &FontId| {
        ui.fonts_mut(|fonts| fonts.layout_no_wrap(text.to_owned(), font.clone(), Color32::PLACEHOLDER).size().x)
    };
    let widest = |column: usize, title: &str, font: &FontId| {
        let texts: BTreeSet<&str> =
            lines.iter().filter_map(|line| line.get(column)).map(|cell| cell.text.as_str()).collect();
        texts.into_iter().map(|text| width(text, font)).fold(width(title, &small), f32::max)
    };
    [
        Column::fixed("State", (DOT + widest(0, "State", &body)).min(STATE_MOST)),
        Column::fixed("Cluster", widest(1, "Cluster", &body).min(CLUSTER_MOST)),
        Column::fill("Target"),
        Column::fixed("Local address", widest(3, "Local address", &theme::mono()).min(ADDRESS_MOST)),
        Column::fill("Pod"),
    ]
}

/// The cells of one forward.
fn cells(row: &ForwardRow, scene: &Scene<'_>) -> [Text; 5] {
    let view = &row.view;
    let (state, tone) = texts::short_state(view, scene.clock);
    let (pod, pod_tone) = texts::pod(view);
    let cluster = scene.view.config.alias(&view.rule.cluster).unwrap_or(view.rule.cluster.as_str()).to_owned();
    [
        Text::new(state).state(tone),
        Text::new(cluster).tone(Tone::Muted),
        Text::new(view.rule.target()),
        Text::new(texts::first_address(view)).mono(),
        Text::new(pod).tone(pod_tone),
    ]
}
