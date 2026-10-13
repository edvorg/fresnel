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
    /// The columns of this type share the width that the fixed columns leave.
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
}

impl Text {
    pub(crate) fn new(text: impl Into<String>) -> Self {
        Self { text: text.into(), tone: Tone::Normal, dot: false, mono: false }
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
    let palette = theme::palette(ui);
    let id = ui.make_persistent_id(id_salt);
    let line = line(ui, ROW_HEIGHT);
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
            ui.add(Label::new(rich).truncate());
        });
    }
}

fn line(ui: &mut Ui, height: f32) -> egui::Rect {
    ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::hover()).0
}

/// The rectangle of each column in a line.
fn cell_rects(line: egui::Rect, columns: &[Column]) -> impl Iterator<Item = egui::Rect> + use<> {
    let fixed_width = |column: &Column| match column.width {
        Width::Fixed(width) => Some(width),
        Width::Fill => None,
    };
    let gaps = GAP * columns.len().saturating_sub(1) as f32;
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
