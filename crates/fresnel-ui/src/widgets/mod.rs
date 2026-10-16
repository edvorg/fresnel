//! Small parts that more than one view uses.

pub(crate) mod chart;
pub(crate) mod mini_table;
pub(crate) mod table;
pub(crate) mod terminal;
pub(crate) mod tone;
pub(crate) mod worker;

use std::sync::Arc;

use egui::text::{LayoutJob, TextWrapping};
use egui::{Color32, FontId, Galley, Id, Rect, Sense, Ui};

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

/// A node of the accessibility tree for a row that a widget paints itself, such as a line of
/// a log or a row of a terminal: a screen reader reads its text, and a test finds the rows by
/// their role. egui gives a place only to the node of a widget that takes the focus, so the
/// row sets its own.
pub(crate) fn row_node(ui: &Ui, rect: Rect, id: Id, text: &str) {
    let response = ui.interact(rect, id, Sense::hover());
    ui.ctx().accesskit_node_builder(response.id, |node| {
        node.set_role(egui::accesskit::Role::ListItem);
        node.set_label(text);
        let (min, max) = (rect.min, rect.max);
        node.set_bounds(egui::accesskit::Rect {
            x0: min.x.into(),
            y0: min.y.into(),
            x1: max.x.into(),
            y1: max.y.into(),
        });
    });
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

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    /// The characters that a galley shows.
    fn shown(galley: &Galley) -> String {
        galley.rows.iter().flat_map(|row| row.glyphs.iter().map(|glyph| glyph.chr)).collect()
    }

    #[test]
    fn a_text_wider_than_its_room_ends_with_a_mark_and_a_shorter_one_stays_whole() {
        let galleys = RefCell::new(Vec::new());
        let mut harness = egui_kittest::Harness::new_ui(|ui| {
            let line = |text: &str| one_line(ui, text, FontId::proportional(13.0), Color32::WHITE, 80.0);
            *galleys.borrow_mut() = vec![line("checkout-api-7d9c5b8f6-gn8jp"), line("gn8jp")];
        });
        harness.run();
        drop(harness);
        let galleys = galleys.into_inner();
        let [long, short] = galleys.as_slice() else { panic!("two galleys") };
        assert!(long.elided && long.size().x <= 80.0, "{:?}", long.size());
        let text = shown(long);
        assert!(text.starts_with("checkout") && text.ends_with('…'), "{text}");
        assert!(!short.elided && shown(short) == "gn8jp");
    }
}
