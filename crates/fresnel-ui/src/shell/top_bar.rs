//! Top bar: the clusters of the kubeconfig files, each with its switch.
//!
//! The selection is a set: the user selects many clusters, and each view shows all of them.

use egui::{Align2, FontId, RichText, Sense, StrokeKind, Ui, WidgetInfo, WidgetType};
use fresnel_core::app::{ClusterRow, Command, Startup};

use super::Scene;
use crate::theme::{self, Palette};
use crate::widgets;

pub(super) const HEIGHT: f32 = 40.0;

pub(super) fn show(ui: &mut Ui, scene: &mut Scene<'_>) {
    let palette = theme::palette(ui);
    ui.horizontal_centered(|ui| {
        ui.add_space(4.0);
        ui.label(RichText::new("fresnel").size(15.0).strong().color(palette.accent));
        ui.add_space(8.0);
        if scene.view.clusters.is_empty() {
            ui.label(RichText::new(startup_text(scene.view.startup)).color(palette.text_muted));
            return;
        }
        select_all(ui, scene, true, "All", "Select all clusters");
        select_all(ui, scene, false, "None", "Select no cluster");
        ui.add_space(4.0);
        egui::ScrollArea::horizontal().auto_shrink(false).show(ui, |ui| {
            for row in &scene.view.clusters {
                let color = scene.colors.color(&row.id, palette);
                if chip(ui, palette, row, color).clicked() {
                    scene.commands.push(Command::Select { cluster: row.id.clone(), selected: !row.selected });
                }
            }
        });
    });
}

/// What the bar shows in place of the clusters while it has none.
pub(crate) fn startup_text(startup: Startup) -> &'static str {
    match startup {
        Startup::Waiting | Startup::Environment => "The login shell gives its environment…",
        Startup::Kubeconfig => "The kubeconfig files load…",
        Startup::Ready => "The kubeconfig files have no context.",
    }
}

/// A button that selects each cluster or no cluster.
fn select_all(ui: &mut Ui, scene: &mut Scene<'_>, selected: bool, title: &str, name: &str) {
    let button = ui.small_button(title).on_hover_text(name);
    widgets::name(&button, name);
    if button.clicked() {
        let changed = scene.view.clusters.iter().filter(|row| row.selected != selected);
        scene.commands.extend(changed.map(|row| Command::Select { cluster: row.id.clone(), selected }));
    }
}

/// One cluster with its color. The chip is a switch: a click selects the cluster or
/// removes it from the selection.
fn chip(ui: &mut Ui, palette: &Palette, row: &ClusterRow, color: egui::Color32) -> egui::Response {
    let name = row.id.as_str();
    let text_color = if row.selected { palette.text } else { palette.text_muted };
    let galley = ui.painter().layout_no_wrap(name.to_owned(), FontId::proportional(12.0), text_color);
    let problem_width = if row.server.is_err() { 12.0 } else { 0.0 };
    let width = 23.0 + galley.size().x + problem_width + 10.0;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 22.0), Sense::click());
    response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, true, row.selected, name));

    let painter = ui.painter();
    let border = if response.hovered() { palette.text_faint } else { palette.border };
    let fill = if row.selected { palette.bg_raised } else { egui::Color32::TRANSPARENT };
    painter.rect(rect, 11.0, fill, (1.0, border), StrokeKind::Inside);
    let mark = rect.left_center() + egui::vec2(13.5, 0.0);
    if row.selected {
        painter.circle_filled(mark, 3.5, color);
    } else {
        painter.circle_stroke(mark, 3.0, (1.0, color));
    }
    let text_pos = egui::pos2(rect.left() + 23.0, rect.center().y - galley.size().y / 2.0);
    let text_right = text_pos.x + galley.size().x;
    painter.galley(text_pos, galley, text_color);
    if row.server.is_err() {
        let pos = egui::pos2(text_right + 5.0, rect.center().y);
        painter.text(pos, Align2::LEFT_CENTER, "!", FontId::proportional(12.0), palette.warn);
    }
    response.on_hover_text(hover_text(row))
}

fn hover_text(row: &ClusterRow) -> String {
    let action = if row.selected { "A click closes its connections." } else { "A click connects it." };
    match &row.server {
        Ok(server) => format!("{}: {server}\n{action}", row.id),
        Err(error) => format!("{}: the app cannot use its kubeconfig entry.\n{error}", row.id),
    }
}
