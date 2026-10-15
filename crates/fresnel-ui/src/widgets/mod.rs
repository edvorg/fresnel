//! Small parts that more than one view uses.

pub(crate) mod chart;
pub(crate) mod mini_table;
pub(crate) mod table;
pub(crate) mod tone;
pub(crate) mod worker;

use egui::{Color32, Sense, Ui};

/// A filled circle that aligns with the text: the mark of a state.
pub(crate) fn dot(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(7.0, 7.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 3.5, color);
}

/// Gives a widget a name in the accessibility tree. Screen readers and the UI tests find a
/// widget by its name. Use it when the text of the widget does not tell its target.
pub(crate) fn name(response: &egui::Response, name: &str) {
    response.ctx.accesskit_node_builder(response.id, |node| node.set_label(name));
}

/// A small bar that a part fills: the use of a limit. A part of 1 or more fills the bar,
/// and a part that is not a number fills nothing.
pub(crate) fn bar(ui: &mut Ui, part: f64, fill: Color32, empty: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(60.0, 6.0), Sense::hover());
    ui.painter().rect_filled(rect, 3.0, empty);
    let part = if part.is_nan() { 0.0 } else { part.clamp(0.0, 1.0) };
    let width = rect.width() * part as f32;
    ui.painter().rect_filled(egui::Rect::from_min_size(rect.min, egui::vec2(width, rect.height())), 3.0, fill);
}
