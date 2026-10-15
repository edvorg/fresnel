//! How a table paints its header, its rows and its cells.

use egui::{Align, Align2, FontId, Label, Layout, Response, RichText, Sense, Shape, Ui, WidgetInfo, WidgetType};

use super::model::{Cell, Column, Lead};
use crate::format;
use crate::theme::{Palette, SMALL};
use crate::widgets::{self, dot};

/// The space between the border of a cell and its text.
const PAD: f32 = 10.0;
/// The space between the mark of a cell and its text.
const GAP: f32 = 6.0;
/// The most characters of a cell that the table lays out: more than the widest column
/// shows. A value of a cluster can be much longer, for example the reason of a status.
pub(crate) const TEXT_LIMIT: usize = 400;

/// The place of the sort arrow beside a title.
const ARROW: f32 = 13.0;
/// The size of the bar of a cell. The hint of the cell gives the values in words.
const BAR: egui::Vec2 = egui::vec2(32.0, 6.0);

/// True for a cell that the scroll regions of `egui_table` cut away. The table lays out
/// such a cell too; a skip saves the time and one more node in the accessibility tree.
pub(super) fn is_clipped(ui: &Ui) -> bool {
    let visible = ui.clip_rect().intersect(ui.max_rect());
    visible.width() < 1.0 || visible.height() < 1.0
}

/// Paints the title of one column. `descending` is set for the sort column. Gives true
/// for a click.
///
/// A double click on the edge of a column makes `egui_table` measure each cell in a pass
/// that sizes only, and gives the column the widest width. A title and a cell then give
/// their full width; else each one would fit into the narrowest column.
pub(super) fn header(ui: &mut Ui, palette: &Palette, column: &Column, descending: Option<bool>) -> bool {
    let rect = ui.max_rect();
    let response = ui.interact(rect, ui.id().with("sort"), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, column.title));
    filler_header(ui, palette);
    let painter = ui.painter();

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
    if ui.is_sizing_pass() {
        // Each column has the place of the arrow, so a sort changes no width.
        ui.set_min_width(PAD + title.width() + ARROW + PAD);
    }
    response.clicked()
}

/// Paints the background of a title. Alone, it is the title of the filler after the last
/// column, so the bar of the titles goes on to the edge of the table.
pub(super) fn filler_header(ui: &Ui, palette: &Palette) {
    let rect = ui.max_rect();
    ui.painter().rect_filled(rect, 0.0, palette.bg_chrome);
    ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, (1.0, palette.border));
}

/// A text in the middle of the place of the table, in place of the rows.
pub(super) fn note(ui: &mut Ui, palette: &Palette, text: &str) {
    ui.centered_and_justified(|ui| ui.label(RichText::new(text).color(palette.text_muted)));
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
        ui.spacing_mut().item_spacing.x = GAP;
        match cell.lead {
            Lead::None => {}
            Lead::Dot(color) => dot(ui, color),
            Lead::Old(since) => old_mark(ui, palette, since),
        }
        if let Some(bar) = &cell.bar {
            widgets::bar(ui, palette, BAR, (bar.part, bar.mark), bar.tone.color(palette));
        }
        let text = format::short(&cell.text, TEXT_LIMIT);
        // The row takes the click, so the text must not take it.
        let label = Label::new(RichText::new(text).color(color)).selectable(false);
        if ui.is_sizing_pass() {
            // The full text and the space after it: the width for a double click (`header`).
            // The gap after the text is a part of that space.
            ui.add(label.extend());
            ui.add_space(PAD - GAP);
        } else {
            let response = ui.add(label.truncate());
            if let Some(hint) = &cell.hint {
                response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, hint.as_ref()));
                response.on_hover_text(hint.as_ref());
            }
        }
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

#[cfg(test)]
mod tests {
    use egui::{Color32, Rect, UiBuilder};
    use egui_kittest::Harness;

    use super::*;
    use crate::theme;

    /// The narrowest column of a table.
    const NARROW: f32 = 48.0;

    /// The width that a title and a cell give in a narrow column, with the text widths of
    /// the title and of the cell.
    fn widths(sizing: bool) -> [f32; 4] {
        let column = Column::new("Last restart", 100.0).numeric();
        let measured = std::cell::Cell::new([0.0; 4]);
        let mut harness = Harness::new_ui(|ui| {
            let palette = theme::palette(ui);
            let place = |ui: &mut Ui| {
                let rect = Rect::from_min_size(ui.cursor().min, egui::vec2(NARROW, 24.0));
                let builder = UiBuilder::new().max_rect(rect);
                ui.new_child(if sizing { builder.sizing_pass() } else { builder })
            };
            let mut title = place(ui);
            header(&mut title, palette, &column, None);
            let text = "CreateContainerConfigError";
            let mut row = place(ui);
            cell(&mut row, palette, &column, &super::super::model::Cell::new(text));
            let width = |text: &str, size| {
                let font = FontId::proportional(size);
                ui.fonts_mut(|fonts| fonts.layout_no_wrap(text.to_owned(), font, Color32::WHITE).size().x)
            };
            let body = egui::TextStyle::Body.resolve(ui.style()).size;
            measured.set([title.min_size().x, width(column.title, SMALL), row.min_size().x, width(text, body)]);
        });
        harness.run();
        drop(harness);
        measured.get()
    }

    #[test]
    fn a_title_and_a_cell_give_their_full_width_only_in_a_pass_that_sizes() {
        let [title, title_text, cell, cell_text] = widths(true);
        assert!((title - (PAD + title_text + ARROW + PAD)).abs() < 0.5, "the title gives {title}");
        assert!((cell - (PAD + cell_text + PAD)).abs() < 0.5, "the cell gives {cell}");

        // Else no part is wider than its column: `egui_table` would make the column wider.
        let [title, _, cell, _] = widths(false);
        assert!(title <= NARROW && cell <= NARROW, "the title gives {title}, the cell {cell}");
    }
}
