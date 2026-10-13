//! How a table paints its header, its rows and its cells.

use egui::{Align, Align2, FontId, Label, Layout, Response, RichText, Sense, Shape, Ui, WidgetInfo, WidgetType};

use super::model::{Cell, Column, Lead};
use crate::format;
use crate::theme::{Palette, SMALL};
use crate::widgets::dot;

/// The space between the border of a cell and its text.
const PAD: f32 = 10.0;
/// The most characters of a cell that the table lays out: more than the widest column
/// shows. A value of a cluster can be much longer, for example the reason of a status.
const TEXT_LIMIT: usize = 400;

/// True for a cell that the scroll regions of `egui_table` cut away. The table lays out
/// such a cell too; a skip saves the time and one more node in the accessibility tree.
pub(super) fn is_clipped(ui: &Ui) -> bool {
    let visible = ui.clip_rect().intersect(ui.max_rect());
    visible.width() < 1.0 || visible.height() < 1.0
}

/// Paints the title of one column. `descending` is set for the sort column. Gives true
/// for a click.
pub(super) fn header(ui: &mut Ui, palette: &Palette, column: &Column, descending: Option<bool>) -> bool {
    let rect = ui.max_rect();
    let response = ui.interact(rect, ui.id().with("sort"), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, column.title));
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, palette.bg_chrome);
    painter.hline(rect.x_range(), rect.bottom() - 0.5, (1.0, palette.border));

    let active = descending.is_some() || response.hovered();
    let color = if active { palette.text } else { palette.text_faint };
    let (anchor, align, inward) = if column.numeric {
        (rect.right_center() - egui::vec2(PAD, 0.0), Align2::RIGHT_CENTER, -1.0)
    } else {
        (rect.left_center() + egui::vec2(PAD, 0.0), Align2::LEFT_CENTER, 1.0)
    };
    let title = painter.text(anchor, align, column.title, FontId::proportional(SMALL), color);
    if let Some(descending) = descending {
        let edge = if column.numeric { title.left() } else { title.right() };
        let center = egui::pos2(edge + inward * 9.0, rect.center().y);
        // The point of the arrow is up for a sequence from small to large.
        let point = if descending { 3.0 } else { -3.0 };
        let arrow =
            vec![center + egui::vec2(0.0, point), center + egui::vec2(-3.5, -point), center + egui::vec2(3.5, -point)];
        painter.add(Shape::convex_polygon(arrow, palette.accent, egui::Stroke::NONE));
    }
    response.clicked()
}

/// Paints the background of a row and gives its click.
pub(super) fn row(ui: &mut Ui, palette: &Palette, selected: bool, is_cursor: bool) -> Response {
    let rect = ui.max_rect();
    let response = ui.interact(rect, ui.id().with("row"), Sense::click());
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, 0.0, palette.accent_bg);
    } else if response.hovered() {
        painter.rect_filled(rect, 0.0, palette.bg_raised.gamma_multiply(0.6));
    }
    if is_cursor {
        painter.rect_filled(rect.with_max_x(rect.left() + 2.0), 0.0, palette.accent);
    }
    painter.hline(rect.x_range(), rect.bottom() - 0.5, (1.0, palette.border.gamma_multiply(0.45)));
    response
}

/// Paints one cell. A text that is too long for its cell is cut and shows in full at
/// the pointer.
pub(super) fn cell(ui: &mut Ui, palette: &Palette, column: &Column, cell: &Cell<'_>) {
    let color = cell.tone.color(palette);
    let layout =
        if column.numeric { Layout::right_to_left(Align::Center) } else { Layout::left_to_right(Align::Center) };
    ui.with_layout(layout, |ui| {
        ui.add_space(PAD);
        ui.spacing_mut().item_spacing.x = 6.0;
        match cell.lead {
            Lead::None => {}
            Lead::Dot(color) => dot(ui, color),
            Lead::Old(since) => old_mark(ui, palette, since),
        }
        let text = format::short(&cell.text, TEXT_LIMIT);
        // The row takes the click, so the text must not take it.
        ui.add(Label::new(RichText::new(text).color(color)).truncate().selectable(false));
    });
}

/// The freshness mark: an open ring in the warning color. A row with it is from a store
/// that is not live, so its values can be old (design rule 4).
pub(crate) fn ring(ui: &mut Ui, palette: &Palette) -> Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(9.0, 9.0), Sense::hover());
    ui.painter().circle_stroke(rect.center(), 3.25, (1.5, palette.warn));
    response
}

/// The freshness mark of one row, with its text for the pointer and for a screen reader.
fn old_mark(ui: &mut Ui, palette: &Palette, since: &str) {
    let response = ring(ui, palette);
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, since));
    response.on_hover_text(since);
}
