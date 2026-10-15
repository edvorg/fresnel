//! A small static table: the channels of a cluster and the rows of the journal.
//!
//! It draws each row that it gets. A list that can be long gives it only the visible rows.

use egui::{Align, Label, Layout, RichText, Sense, Ui, UiBuilder};

use super::dot;
use super::tone::Tone;
use crate::theme::{self, ROW_HEIGHT, SMALL};

#[derive(Clone, Copy)]
pub(crate) enum Width {
    Fixed(f32),
    /// This part of the width of the line, less its gaps: the column fits a long text in a
    /// wide line and leaves room for the other columns in a narrow one.
    Part(f32),
    /// The columns of this type share the width that the other columns leave.
    Fill,
}

pub(crate) struct Column {
    pub(crate) title: &'static str,
    pub(crate) width: Width,
    /// A number aligns to the right.
    pub(crate) numeric: bool,
}

impl Column {
    pub(crate) const fn fixed(title: &'static str, width: f32) -> Self {
        Self { title, width: Width::Fixed(width), numeric: false }
    }

    pub(crate) const fn part(title: &'static str, part: f32) -> Self {
        Self { title, width: Width::Part(part), numeric: false }
    }

    pub(crate) const fn fill(title: &'static str) -> Self {
        Self { title, width: Width::Fill, numeric: false }
    }

    pub(crate) const fn numeric(mut self) -> Self {
        self.numeric = true;
        self
    }
}

/// The content of one cell.
#[derive(Clone, Debug)]
pub(crate) struct Text {
    pub(crate) text: String,
    pub(crate) tone: Tone,
    /// A state dot in the tone color before the text.
    pub(crate) dot: bool,
    /// A clock time or an other text with fixed columns.
    pub(crate) mono: bool,
    /// More words at the pointer, as the reason of the server for a state.
    pub(crate) hover: Option<String>,
}

impl Text {
    pub(crate) fn new(text: impl Into<String>) -> Self {
        Self { text: text.into(), tone: Tone::Normal, dot: false, mono: false, hover: None }
    }

    pub(crate) fn tone(mut self, tone: Tone) -> Self {
        self.tone = tone;
        self
    }

    pub(crate) fn state(mut self, tone: Tone) -> Self {
        self.tone = tone;
        self.dot = true;
        self
    }

    pub(crate) fn mono(mut self) -> Self {
        self.mono = true;
        self
    }

    pub(crate) fn hover(mut self, hover: String) -> Self {
        self.hover = Some(hover);
        self
    }
}

const GAP: f32 = 12.0;

/// Draws the line with the column titles.
pub(crate) fn header(ui: &mut Ui, id_salt: impl egui::AsIdSalt, columns: &[Column]) {
    let palette = theme::palette(ui);
    let id = ui.make_persistent_id(id_salt);
    let line = line(ui, ROW_HEIGHT - 2.0);
    ui.painter().hline(line.x_range(), line.bottom() - 0.5, (1.0, palette.border));
    for (index, (column, rect)) in columns.iter().zip(cell_rects(line, columns)).enumerate() {
        let title = RichText::new(column.title).size(SMALL).color(palette.text_faint);
        cell(ui, id.with(index), rect, column.numeric, |ui| {
            ui.add(Label::new(title).truncate().selectable(false));
        });
    }
}

/// Draws one row. A cell that is too narrow for its text cuts the text and shows all of
/// it at the pointer.
pub(crate) fn row(ui: &mut Ui, id_salt: impl egui::AsIdSalt, columns: &[Column], cells: &[Text]) {
    let line = line(ui, ROW_HEIGHT);
    cells_of(ui, id_salt, line, columns, cells);
}

/// Draws one row that the user can choose with a click, with the background of a chosen
/// row. `name` is its name in the accessibility tree. Gives the response of the row.
pub(crate) fn row_to_choose(
    ui: &mut Ui,
    id_salt: impl egui::AsIdSalt + Copy,
    columns: &[Column],
    cells: &[Text],
    (chosen, name): (bool, &str),
) -> egui::Response {
    let palette = theme::palette(ui);
    let id = ui.make_persistent_id(id_salt).with("row");
    let line = ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW_HEIGHT), Sense::hover()).0;
    let response = ui.interact(line, id, Sense::click());
    super::name(&response, name);
    if chosen {
        ui.painter().rect_filled(line, 0.0, palette.accent_bg);
    } else if response.hovered() {
        ui.painter().rect_filled(line, 0.0, palette.bg_raised);
    }
    cells_of(ui, id_salt, line, columns, cells);
    response
}

fn cells_of(ui: &mut Ui, id_salt: impl egui::AsIdSalt, line: egui::Rect, columns: &[Column], cells: &[Text]) {
    let palette = theme::palette(ui);
    let id = ui.make_persistent_id(id_salt);
    ui.painter().hline(line.x_range(), line.bottom() - 0.5, (1.0, palette.border.gamma_multiply(0.45)));
    for (index, ((column, rect), text)) in columns.iter().zip(cell_rects(line, columns)).zip(cells).enumerate() {
        let color = text.tone.color(palette);
        cell(ui, id.with(index), rect, column.numeric, |ui| {
            if text.dot {
                dot(ui, color);
            }
            let mut rich = RichText::new(&text.text).color(color);
            if text.mono {
                rich = rich.font(theme::mono());
            }
            let label = ui.add(Label::new(rich).truncate());
            if let Some(hover) = &text.hover {
                label.on_hover_text(hover);
            }
        });
    }
}

fn line(ui: &mut Ui, height: f32) -> egui::Rect {
    ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::hover()).0
}

/// The rectangle of each column in a line.
fn cell_rects(line: egui::Rect, columns: &[Column]) -> impl Iterator<Item = egui::Rect> + use<> {
    let gaps = GAP * columns.len().saturating_sub(1) as f32;
    let fixed_width = |column: &Column| match column.width {
        Width::Fixed(width) => Some(width),
        Width::Part(part) => Some(part * (line.width() - gaps).max(0.0)),
        Width::Fill => None,
    };
    let fixed: f32 = columns.iter().filter_map(fixed_width).sum();
    let fills = columns.iter().filter(|column| fixed_width(column).is_none()).count().max(1);
    let fill = ((line.width() - gaps - fixed) / fills as f32).max(60.0);
    let widths: Vec<f32> = columns.iter().map(|column| fixed_width(column).unwrap_or(fill)).collect();
    let mut left = line.left();
    widths.into_iter().map(move |width| {
        let rect = egui::Rect::from_min_max(egui::pos2(left, line.top()), egui::pos2(left + width, line.bottom()));
        left += width + GAP;
        rect
    })
}

/// Runs `add` in a child `Ui` that is clipped to `rect`. The parent layout does not move.
fn cell(ui: &mut Ui, id: egui::Id, rect: egui::Rect, numeric: bool, add: impl FnOnce(&mut Ui)) {
    let layout = if numeric { Layout::right_to_left(Align::Center) } else { Layout::left_to_right(Align::Center) };
    let mut child = ui.new_child(UiBuilder::new().id_salt(id).max_rect(rect).layout(layout));
    child.shrink_clip_rect(rect);
    child.spacing_mut().item_spacing.x = 6.0;
    add(&mut child);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_part_column_has_its_part_of_the_line_and_a_fill_column_the_rest() {
        let columns = [Column::fill("Pod"), Column::part("Verdict", 0.25), Column::fixed("State", 100.0)];
        let widths = |width: f32| {
            let line = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 20.0));
            cell_rects(line, &columns).map(|rect| rect.width()).collect::<Vec<f32>>()
        };
        // 424 points less the gaps of 12 points: 400.
        assert_eq!(widths(424.0), [200.0, 100.0, 100.0]);
        assert_eq!(widths(824.0), [500.0, 200.0, 100.0], "a wide line widens the part and the fill");
    }
}
