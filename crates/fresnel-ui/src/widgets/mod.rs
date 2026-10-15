//! Small parts that more than one view uses.

pub(crate) mod chart;
pub(crate) mod mini_table;
pub(crate) mod table;
pub(crate) mod tone;
pub(crate) mod worker;

use std::sync::Arc;

use egui::text::{LayoutJob, TextWrapping};
use egui::{Color32, FontId, Galley, Sense, Ui};

use crate::theme::Palette;

/// A text on one line that ends with "…" where it is wider than `width`. A cut with no
/// mark makes a part of a name look like the whole name: three pods with one name.
pub(crate) fn one_line(ui: &Ui, text: &str, font: FontId, color: Color32, width: f32) -> Arc<Galley> {
    let mut job = LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = TextWrapping::truncate_at_width(width.max(0.0));
    ui.painter().layout_job(job)
}

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

/// A bar that a part fills: the use of a limit. `mark` is a line across it, for example the
/// request of a pod on the bar of its limit. A part of 1 or more fills the bar, and a part
/// that is not a number fills nothing. The words of the values are the work of the caller.
pub(crate) fn bar(ui: &mut Ui, palette: &Palette, size: egui::Vec2, (part, mark): (f32, Option<f32>), fill: Color32) {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let (painter, radius) = (ui.painter(), size.y / 2.0);
    painter.rect_filled(rect, radius, palette.bg_raised);
    let clamped = |part: f32| if part.is_nan() { 0.0 } else { part.clamp(0.0, 1.0) };
    painter.rect_filled(rect.with_max_x(rect.left() + rect.width() * clamped(part)), radius, fill);
    if let Some(mark) = mark {
        let x = rect.left() + rect.width() * clamped(mark);
        painter.vline(x, rect.y_range().expand(2.0), (1.5, palette.text_muted));
    }
}
