//! What the table draws in one frame for `egui_table`: the titles, the rows and the cells
//! of the columns that show. A position of `egui_table` is a place on the screen; this
//! module turns it into the position of the column in the model. Only `table.rs` names
//! that crate: it gives each call of it to the functions of this module.

use std::collections::HashMap;

use egui::{Id, Modifiers, Rect, Ui};

use super::cells;
use super::edges::Seen;
use super::model::{Column, Model, ObjectId, Sort};
use super::selection::Selection;
use crate::theme::Palette;

pub(super) struct Delegate<'a, M: Model> {
    pub(super) model: &'a M,
    pub(super) rows: &'a [M::Row],
    /// The identity of the table in egui.
    pub(super) table_id: Id,
    /// The widgets of each object in this frame. `egui_table` draws a row in the part that
    /// scrolls to the side and then in the fixed first column, in that sequence in each frame.
    pub(super) drawn: HashMap<&'a ObjectId, u8>,
    /// The part of the place of the table that shows: no part above it paints or takes a press.
    pub(super) shown: Rect,
    pub(super) scene: &'a M::Scene,
    pub(super) palette: &'static Palette,
    pub(super) columns: &'a [Column],
    /// The position in the model of each column that shows, in the sequence of the screen.
    pub(super) visible: &'a [usize],
    pub(super) sort: Sort,
    pub(super) selection: &'a Selection,
    /// The width of each column that shows in this frame.
    pub(super) widths: &'a [f32],
    /// The column of the model whose title the user clicked.
    pub(super) header_clicked: Option<usize>,
    pub(super) row_clicked: Option<(usize, Modifiers)>,
    /// Where the title of the filler starts on the screen: the end of the columns.
    pub(super) columns_end: Option<f32>,
    /// The edges, the titles and the rows of the frame, for the handles.
    pub(super) seen: Seen,
    /// The column of the model that a double click fits, and the widest part of it in the
    /// pass that measures it.
    pub(super) fit: Option<usize>,
    pub(super) fitted: f32,
    /// Draws the menu of the row of an object.
    pub(super) menu: &'a mut dyn FnMut(&mut Ui, &ObjectId),
    /// The height of a row.
    pub(super) row_height: f32,
}

impl<'a, M: Model> Delegate<'a, M> {
    /// The row with this number. New data can arrive between two frames, so no number
    /// of `egui_table` is an index with no check.
    fn row(&self, number: u64) -> Option<(usize, &'a M::Row)> {
        let index = usize::try_from(number).ok()?;
        Some((index, self.rows.get(index)?))
    }

    /// The column of the model at this position of the screen, with its description.
    fn column(&self, position: usize) -> Option<(usize, Column)> {
        let index = *self.visible.get(position)?;
        Some((index, *self.columns.get(index)?))
    }

    /// Takes the width that a title or a cell of the column `fit` needs, in the pass that
    /// measures that column.
    fn measure(&mut self, ui: &Ui, column: usize) {
        if self.fit == Some(column) && ui.is_sizing_pass() {
            self.fitted = self.fitted.max(ui.min_size().x);
        }
    }
}

impl<M: Model> Delegate<'_, M> {
    /// The title of the column at this position of the screen.
    pub(super) fn header(&mut self, ui: &mut Ui, position: usize) {
        ui.shrink_clip_rect(self.shown);
        let column = self.column(position);
        if let Some((index, _)) = column {
            self.seen.title(ui, index, self.widths.get(position).copied());
        }
        if position == self.visible.len() {
            // `egui_table` lays out a title in the fixed part and in the part that scrolls
            // to the side. The part that scrolls gives the smaller place.
            let left = ui.max_rect().left();
            self.columns_end = Some(self.columns_end.map_or(left, |end| end.min(left)));
        }
        if cells::is_clipped(ui) {
            return;
        }
        let Some((index, column)) = column else {
            cells::filler_header(ui, self.palette);
            return;
        };
        let descending = (self.sort.column == index).then_some(self.sort.descending);
        if cells::header(ui, self.palette, &column, descending) {
            self.header_clicked = Some(index);
        }
        self.measure(ui, index);
    }

    /// The background of a row, its click and its menu.
    ///
    /// The widget of a row has the identity of its object, not the number of its row: egui
    /// gives a press, its click and an open menu to that identity. With the number, rows that
    /// the worker sorted or changed between a press and its release gave the click, or the
    /// action of an open menu, to the object that came to that row.
    pub(super) fn row_background(&mut self, ui: &mut Ui, row_nr: u64) {
        let Some((index, row)) = self.row(row_nr) else {
            return;
        };
        ui.shrink_clip_rect(self.shown);
        let id = self.model.id(row);
        let part = self.drawn.entry(id).or_default();
        let widget = self.table_id.with(("row", id, *part));
        *part = part.saturating_add(1);
        let (selected, is_cursor) = (self.selection.contains(id), self.selection.cursor() == Some(id));
        self.seen.row(ui);
        let response = cells::row(ui, widget, self.palette, selected, is_cursor);
        if response.clicked() {
            self.row_clicked = Some((index, ui.input(|input| input.modifiers)));
        }
        response.context_menu(|ui| (self.menu)(ui, id));
    }

    /// The cell of a row at this position of the screen.
    pub(super) fn cell(&mut self, ui: &mut Ui, row_nr: u64, position: usize) {
        ui.shrink_clip_rect(self.shown);
        if cells::is_clipped(ui) {
            return;
        }
        let (Some((_, row)), Some((index, column))) = (self.row(row_nr), self.column(position)) else {
            return;
        };
        cells::cell(ui, self.palette, &column, &self.model.cell(row, index, self.scene));
        self.measure(ui, index);
    }
}
