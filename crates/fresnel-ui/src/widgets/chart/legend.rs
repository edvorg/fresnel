//! The legend of a chart: a dot and the name of each line, and a dash for each fixed line,
//! in a row that the caller chooses, as the line of the heading of the chart. The legend of
//! `egui_plot` lies over the plot: it hid the newest samples, and the lines ran through its
//! names. A click on the name of a line hides the line or shows it again, as in that legend.

use std::collections::BTreeSet;

use egui::{Button, Color32, Id, RichText, Sense, Shape, Stroke, Ui};

use super::{Line, Mark};
use crate::theme::Palette;
use crate::widgets::dot;

/// The space between two entries.
const GAP: f32 = 10.0;

/// The names of the lines that the user hid in the chart `id`.
pub(super) fn hidden(ui: &Ui, id: Id) -> BTreeSet<String> {
    ui.data(|data| data.get_temp(id.with("hidden lines"))).unwrap_or_default()
}

/// Draws the legend of the chart `id`. The chart reads the hidden lines in the same frame.
pub(super) fn show(ui: &mut Ui, palette: &Palette, id: Id, (lines, marks): (&[Line], &[Mark])) {
    let mut hidden = hidden(ui, id);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for line in lines {
            let shown = !hidden.contains(&line.name);
            dot(ui, if shown { line.color } else { palette.text_faint });
            let text = RichText::new(&line.legend).color(if shown { palette.text } else { palette.text_faint });
            let hint = format!("{} the line {}", if shown { "Hides" } else { "Shows" }, line.name);
            if ui.add(Button::new(text).frame(false)).on_hover_text(hint).clicked() && !hidden.remove(&line.name) {
                hidden.insert(line.name.clone());
            }
            ui.add_space(GAP);
        }
        for mark in marks {
            dash(ui, mark.color);
            ui.label(RichText::new(&mark.name).color(palette.text_muted));
            ui.add_space(GAP);
        }
    });
    ui.data_mut(|data| data.insert_temp(id.with("hidden lines"), hidden));
}

/// A short dashed line in the color of a fixed line, as the chart draws it.
fn dash(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 7.0), Sense::hover());
    let ends = [rect.left_center(), rect.right_center()];
    ui.painter().extend(Shape::dashed_line(&ends, Stroke::new(1.5, color), 3.0, 2.0));
}
