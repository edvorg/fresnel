//! The table of the objects of one kind, on `egui_table` (plan section 11.2). No other
//! module uses that crate, so an upgrade of it changes this file only.
//!
//! `egui_table` gives the virtual rows, the fixed header and the widths of the columns.
//! This module adds the rows from a worker thread, the sort, the cursor and the selection
//! by the identity of an object, and the keys.
//!
//! - `model`: what a kind gives to its table. `worker`: the thread that computes the rows.
//! - `rows`: a request and its rows. `supply`: the last request and the rows on the screen.
//! - `cells`: the paint of each part. `keys`: the keys.
//! - `selection`: the selected objects. `select`: what a click and a key do to them.

pub(crate) mod cells;
mod keys;
pub(crate) mod model;
mod rows;
mod select;
mod selection;
mod supply;
pub(crate) mod worker;

use std::sync::Arc;
use std::time::Duration;

use egui::{EventFilter, Modifiers, RichText, Sense, Ui};
use fresnel_core::net::task::Tasks;

use self::model::{Column, Model, ObjectId, Sort};
pub(crate) use self::rows::Rows;
use self::select::{click, forget_gone, key, position};
pub(crate) use self::selection::Selection;
pub(crate) use self::supply::Progress;
use self::supply::Supply;
use self::worker::Wake;
use crate::theme::{self, Palette, ROW_HEIGHT};

/// What the user did to the cursor of a table in one frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Act {
    /// A click or a key put the cursor on a row.
    Cursor,
    /// Escape removed the selection and the cursor.
    Cleared,
}

/// The table of one kind. It owns its worker thread.
pub(crate) struct Table<M: Model> {
    /// The title of the kind. It names the worker thread and the saved column widths.
    name: &'static str,
    model: Arc<M>,
    supply: Supply<M>,
    selection: Selection,
    /// The object whose row the next frame scrolls to.
    reveal: Option<ObjectId>,
}

impl<M: Model> Table<M> {
    /// Starts the worker thread of the table. `wake` asks for a repaint for a result.
    pub(crate) fn new(tasks: &Tasks, wake: Wake, name: &'static str, model: M) -> Self {
        let model = Arc::new(model);
        let supply = Supply::start(tasks, wake, name, Arc::clone(&model));
        Self { name, model, supply, selection: Selection::default(), reveal: None }
    }

    /// Tells the table what its rows come from in this frame, and takes a result of the
    /// worker. `data_serial` grows when the objects change, and `data` gives them.
    pub(crate) fn request(&mut self, filter: &M::Filter, data_serial: u64, data: impl FnOnce() -> M::Data) {
        self.supply.request(filter, data_serial, data);
    }

    pub(crate) fn progress(&self) -> Progress<'_> {
        self.supply.progress()
    }

    /// The rows on the screen. `None` before the first result.
    pub(crate) fn rows(&self) -> Option<&Rows<M>> {
        self.supply.rows()
    }

    /// The time that the worker thread took for the rows on the screen.
    pub(crate) fn compute_time(&self) -> Option<Duration> {
        self.supply.compute_time()
    }

    pub(crate) fn selection(&self) -> &Selection {
        &self.selection
    }

    /// Removes the objects that `gone` names from the selection.
    pub(crate) fn forget(&mut self, gone: impl Fn(&ObjectId) -> bool) {
        self.selection.forget(gone);
    }

    pub(crate) fn clear_selection(&mut self) {
        self.selection.clear();
    }

    /// Gives up the rows, the objects of the last request and the selection. A view
    /// calls this in each frame that does not paint it: it must hold no rows then (plan
    /// section 7.3), and no frame would remove an object that goes away from the selection.
    pub(crate) fn rest(&mut self) {
        self.supply.rest();
        self.selection.clear();
        self.reveal = None;
    }

    /// The row of the cursor, if the rows on the screen have its object.
    pub(crate) fn cursor_row(&self) -> Option<&M::Row> {
        let (rows, cursor) = (&self.rows()?.rows, self.selection.cursor()?);
        rows.iter().find(|row| self.model.id(row) == cursor)
    }

    /// Draws the table into the rest of `ui`. `empty` tells why the table has no row.
    /// Gives what the user did to the cursor.
    pub(crate) fn show(&mut self, ui: &mut Ui, scene: &M::Scene, empty: impl FnOnce() -> String) -> Option<Act> {
        let palette = theme::palette(ui);
        if let Some(rest) = self.supply.time_to_slow() {
            // The frame after this time tells that the table waits for its rows.
            ui.ctx().request_repaint_after(rest);
        }
        let Some(shown) = self.supply.rows() else {
            let text = match self.supply.failure() {
                Some(failure) => format!("The rows are not computed: the worker thread {failure}"),
                None => "The rows are computed…".to_owned(),
            };
            note(ui, palette, &text);
            return None;
        };
        let rows = shown.rows.as_slice();
        if rows.is_empty() {
            note(ui, palette, &empty());
            return None;
        }
        let (model, columns) = (&*self.model, self.model.columns());
        let rect = ui.available_rect_before_wrap();
        // The focus of the keys is an area below the rows. The rows are above it and
        // get the clicks.
        let focus_id = ui.make_persistent_id((self.name, "keys"));
        let focus = ui.interact(rect, focus_id, Sense::focusable_noninteractive());

        let table_columns: Vec<egui_table::Column> =
            columns.iter().enumerate().map(|(i, c)| self.column(i, c)).collect();
        let mut table = egui_table::Table::new()
            .id_salt(self.name)
            .num_rows(rows.len() as u64)
            .columns(table_columns.clone())
            .num_sticky_cols(1)
            .headers([egui_table::HeaderRow::new(ROW_HEIGHT + 2.0)])
            .auto_size_mode(egui_table::AutoSizeMode::Never);
        if let Some(row) = self.reveal.take().and_then(|target| position(model, rows, &target)) {
            table = table.scroll_to_row(row as u64, None);
        }
        prepare_widths(ui, table.get_id(ui), &table_columns, rect.width());
        let mut delegate = Delegate {
            model,
            rows,
            scene,
            palette,
            columns,
            sort: self.supply.sort(),
            selection: &self.selection,
            header_clicked: None,
            row_clicked: None,
        };
        table.show(ui, &mut delegate);
        let (header_clicked, row_clicked) = (delegate.header_clicked, delegate.row_clicked);

        let (mut acted, mut by_key) = (None, false);
        if let Some((row, modifiers)) = row_clicked {
            click(model, rows, &mut self.selection, row, modifiers);
            ui.memory_mut(|memory| memory.request_focus(focus_id));
            acted = Some(Act::Cursor);
        }
        if focus.has_focus() {
            // With this filter, the arrows and Escape are keys of the table. Without it,
            // egui moves the focus to some other widget.
            let filter = EventFilter { vertical_arrows: true, escape: true, ..EventFilter::default() };
            ui.memory_mut(|memory| memory.set_focus_lock_filter(focus_id, filter));
            let page_rows = (rect.height() / ROW_HEIGHT).floor() as usize;
            if let Some((nav, extend)) = keys::read(ui, page_rows) {
                acted = key(model, rows, &mut self.selection, nav, extend);
                by_key = true;
            }
        }
        if (row_clicked.is_some() || by_key)
            && let Some((rows, objects)) = self.supply.older_rows()
        {
            forget_gone(model, rows, objects, &mut self.selection);
        }
        if by_key {
            // The next frame scrolls to the row of the cursor.
            self.reveal = self.selection.cursor().filter(|_| acted == Some(Act::Cursor)).cloned();
            ui.ctx().request_repaint();
        }
        if let Some(column) = header_clicked {
            // The next frame asks the worker for the rows of the new sort.
            self.supply.set_sort(self.supply.sort().clicked(column));
            ui.ctx().request_repaint();
        }
        acted
    }

    /// The column of `egui_table` for a column of the model. The first column takes the
    /// width that the other ones leave, so the user cannot change its width.
    fn column(&self, index: usize, column: &Column) -> egui_table::Column {
        egui_table::Column::new(column.width)
            .range(48.0..=2000.0)
            .resizable(index > 0)
            // With an identity, a column keeps its width when the columns change.
            .id(egui::Id::new((self.name, column.title)))
    }
}

/// A text in the middle of the place of the table.
fn note(ui: &mut Ui, palette: &Palette, text: &str) {
    ui.centered_and_justified(|ui| ui.label(RichText::new(text).color(palette.text_muted)));
}

/// The width of the scroll bar of the table.
const SCROLL_BAR: f32 = 12.0;

/// Stores the widths of the columns before `egui_table` reads them. That crate gives a
/// column that it does not know the width of its content, so the layout would follow the
/// rows on the screen. The first column takes the width that the other ones leave, but
/// not less than its own: a narrow table scrolls to the side.
fn prepare_widths(ui: &Ui, table_id: egui::Id, columns: &[egui_table::Column], available: f32) {
    let mut state = egui_table::TableState::load(ui.ctx(), table_id).unwrap_or_default();
    let mut others = 0.0;
    for (index, column) in columns.iter().enumerate().skip(1) {
        others += *state.col_widths.entry(column.id_for(index)).or_insert(column.current);
    }
    if let Some(first) = columns.first() {
        state.col_widths.insert(first.id_for(0), first.current.max(available - others - SCROLL_BAR));
    }
    state.store(ui.ctx(), table_id);
}

struct Delegate<'a, M: Model> {
    model: &'a M,
    rows: &'a [M::Row],
    scene: &'a M::Scene,
    palette: &'static Palette,
    columns: &'static [Column],
    sort: Sort,
    selection: &'a Selection,
    header_clicked: Option<usize>,
    row_clicked: Option<(usize, Modifiers)>,
}

impl<M: Model> Delegate<'_, M> {
    /// The row with this number. New data can arrive between two frames, so no number
    /// of `egui_table` is an index with no check.
    fn row(&self, number: u64) -> Option<(usize, &M::Row)> {
        let index = usize::try_from(number).ok()?;
        Some((index, self.rows.get(index)?))
    }
}

impl<M: Model> egui_table::TableDelegate for Delegate<'_, M> {
    fn header_cell_ui(&mut self, ui: &mut Ui, cell: &egui_table::HeaderCellInfo) {
        let index = cell.col_range.start;
        let Some(column) = self.columns.get(index).filter(|_| !cells::is_clipped(ui)) else {
            return;
        };
        let descending = (self.sort.column == index).then_some(self.sort.descending);
        if cells::header(ui, self.palette, column, descending) {
            self.header_clicked = Some(index);
        }
    }

    fn row_ui(&mut self, ui: &mut Ui, row_nr: u64) {
        let Some((index, row)) = self.row(row_nr) else {
            return;
        };
        let id = self.model.id(row);
        let (selected, is_cursor) = (self.selection.contains(id), self.selection.cursor() == Some(id));
        if cells::row(ui, self.palette, selected, is_cursor).clicked() {
            self.row_clicked = Some((index, ui.input(|input| input.modifiers)));
        }
    }

    fn cell_ui(&mut self, ui: &mut Ui, cell: &egui_table::CellInfo) {
        if cells::is_clipped(ui) {
            return;
        }
        let (Some((_, row)), Some(column)) = (self.row(cell.row_nr), self.columns.get(cell.col_nr)) else {
            return;
        };
        cells::cell(ui, self.palette, column, &self.model.cell(row, cell.col_nr, self.scene));
    }

    fn default_row_height(&self) -> f32 {
        ROW_HEIGHT
    }
}

#[cfg(test)]
mod tests;
