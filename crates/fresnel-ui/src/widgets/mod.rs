//! Small parts that more than one view uses.

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
