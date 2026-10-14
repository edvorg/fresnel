//! The table of the objects of one kind, on `egui_table` (plan section 11.2). No other
//! module uses that crate, so an upgrade of it changes this file only.
//!
//! `egui_table` gives the virtual rows and the fixed header. This module adds the rows from
//! a worker thread, the sort, the cursor and the selection by the identity of an object,
//! the keys, the widths of the columns and the handles that change them.
//!
//! - `model`: what a kind gives to its table. `worker`: the thread that computes the rows.
//! - `rows`: a request and its rows. `supply`: the last request and the rows on the screen.
//! - `cells`: the paint of each part. `keys`: the keys. `widths`: the widths of the columns.
//! - `edges`: the handle at the right of each column, for a drag and a double click.
//! - `selection`: the selected objects. `select`: what a click and a key do to them.

pub(crate) mod cells;
mod edges;
mod keys;
pub(crate) mod model;
mod rows;
mod select;
mod selection;
mod supply;
mod widths;
pub(crate) mod worker;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use egui::{EventFilter, Modifiers, Sense, Ui};
use fresnel_core::app::Command;
use fresnel_core::net::task::Tasks;
use fresnel_core::state_file::TableLayout;

use self::edges::{Edge, Edges};
use self::model::{Column, Model, ObjectId, Sort};
pub(crate) use self::rows::Rows;
use self::select::{click, forget_gone, key, position};
pub(crate) use self::selection::Selection;
pub(crate) use self::supply::Progress;
use self::supply::Supply;
use self::widths::{FrameWidths, NARROWEST, WIDEST, Widths};
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
    widths: Widths,
    edges: Edges,
    /// The layout that the core has of this table, as far as the table knows. `None` before
    /// the core read the state file.
    saved: Option<TableLayout>,
}

impl<M: Model> Table<M> {
    /// Starts the worker thread of the table. `wake` asks for a repaint for a result.
    pub(crate) fn new(tasks: &Tasks, wake: Wake, name: &'static str, model: M) -> Self {
        let model = Arc::new(model);
        let supply = Supply::start(tasks, wake, name, Arc::clone(&model));
        let widths = Widths::new(model.columns());
        let (selection, edges) = (Selection::default(), Edges::default());
        Self { name, model, supply, selection, reveal: None, widths, edges, saved: None }
    }

    /// Takes the saved layout of the table one time, when the core read the state file
    /// (`CoreView::tables`). A width that the user changed before that stays.
    pub(crate) fn restore(&mut self, tables: Option<&BTreeMap<String, TableLayout>>) {
        let Some(tables) = tables.filter(|_| self.saved.is_none()) else {
            return;
        };
        let layout = tables.get(self.name).cloned().unwrap_or_default();
        if !self.widths.changed() {
            self.widths.restore(self.model.columns(), &layout);
        }
        self.saved = Some(layout);
    }

    /// The command that saves the widths that the user changed. It comes one time, after
    /// the end of the drag, and only after `restore`: the core does not write the file
    /// before it read it.
    pub(crate) fn save_layout(&mut self, ui: &Ui) -> Option<Command> {
        let saved = self.saved.as_mut()?;
        if ui.input(|input| input.pointer.any_down()) {
            return None;
        }
        let layout = self.widths.take_layout(self.model.columns()).filter(|layout| layout != saved)?;
        saved.clone_from(&layout);
        Some(Command::SaveTable { table: self.name.to_owned(), layout })
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
        self.edges.rest();
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
            cells::note(ui, palette, &text);
            return None;
        };
        let rows = shown.rows.as_slice();
        if rows.is_empty() {
            cells::note(ui, palette, &empty());
            return None;
        }
        let (model, columns) = (&*self.model, self.model.columns());
        let rect = ui.available_rect_before_wrap();
        // The focus of the keys is an area below the rows. The rows are above it and
        // get the clicks.
        let focus_id = ui.make_persistent_id((self.name, "keys"));
        let focus = ui.interact(rect, focus_id, Sense::focusable_noninteractive());

        // The vertical scroll bar takes its place at the right of the rows.
        let place = rect.width() - ui.spacing().scroll.allocated_width();
        if let Some((column, width)) = self.edges.dragged(ui) {
            self.widths.drag(column, width, place);
        }
        let fit = self.edges.take_fit();
        let frame = self.widths.for_frame(place);
        let table_columns = self.table_columns(&frame, fit);
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
        let table_id = table.get_id(ui);
        write_widths(ui, table_id, &table_columns);
        let mut delegate = Delegate {
            model,
            rows,
            scene,
            palette,
            columns,
            sort: self.supply.sort(),
            selection: &self.selection,
            widths: &frame.columns,
            header_clicked: None,
            row_clicked: None,
            columns_end: None,
            edges: Vec::new(),
            header_top: None,
            fit,
            fitted: 0.0,
        };
        table.show(ui, &mut delegate);
        let (header_clicked, row_clicked) = (delegate.header_clicked, delegate.row_clicked);
        if let Some(end) = delegate.columns_end {
            self.widths.shown(&frame, end - rect.left());
        }
        if let Some(column) = fit.filter(|_| delegate.fitted > 0.0) {
            self.widths.set(column, delegate.fitted);
        }
        if let Some(top) = delegate.header_top {
            let id = |column: usize| ui.make_persistent_id((self.name, columns.get(column).map(|c| c.title), "edge"));
            self.edges.handles(ui, &delegate.edges, (top, rect.bottom()), id, &frame.columns);
        }

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
        // The worker can be at work on newer rows. Then a row of this frame can be the
        // row of an object that went away, and the selection must not keep it.
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

    /// The columns of `egui_table`: one for each column of the model, with the widths of
    /// this frame, and the filler after them. The table changes a width, not `egui_table`:
    /// it measures the cells of the column `fit` only.
    fn table_columns(&self, frame: &FrameWidths, fit: Option<usize>) -> Vec<egui_table::Column> {
        let columns = self.model.columns().iter().zip(&frame.columns).enumerate().map(|(index, (column, width))| {
            // The identity keys the width in the state of `egui_table`.
            egui_table::Column::new(*width)
                .range(NARROWEST..=WIDEST)
                .resizable(false)
                .auto_size_this_frame(fit == Some(index))
                .id(egui::Id::new((self.name, column.title)))
        });
        let filler = egui_table::Column::new(frame.filler)
            .range(0.0..=f32::INFINITY)
            .resizable(false)
            .id(egui::Id::new((self.name, "filler")));
        columns.chain([filler]).collect()
    }
}

/// Gives `egui_table` the width of each column before it reads them. That crate keeps
/// the widths in its own state, and it would give a column that it does not know the
/// width of its content: the layout would then follow the rows on the screen.
fn write_widths(ui: &Ui, table_id: egui::Id, columns: &[egui_table::Column]) {
    let mut state = egui_table::TableState::load(ui.ctx(), table_id).unwrap_or_default();
    for (index, column) in columns.iter().enumerate() {
        state.col_widths.insert(column.id_for(index), column.current);
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
    /// The width of each column in this frame.
    widths: &'a [f32],
    header_clicked: Option<usize>,
    row_clicked: Option<(usize, Modifiers)>,
    /// Where the title of the filler starts on the screen: the end of the columns.
    columns_end: Option<f32>,
    /// The edge of each title that shows, and the top of the titles.
    edges: Vec<Edge>,
    header_top: Option<f32>,
    /// The column that a double click fits, and the widest part of it in the pass that
    /// measures it.
    fit: Option<usize>,
    fitted: f32,
}

impl<M: Model> Delegate<'_, M> {
    /// The row with this number. New data can arrive between two frames, so no number
    /// of `egui_table` is an index with no check.
    fn row(&self, number: u64) -> Option<(usize, &M::Row)> {
        let index = usize::try_from(number).ok()?;
        Some((index, self.rows.get(index)?))
    }

    /// Takes the width that a title or a cell of the column `fit` needs, in the pass that
    /// measures that column.
    fn measure(&mut self, ui: &Ui, column: usize) {
        if self.fit == Some(column) && ui.is_sizing_pass() {
            self.fitted = self.fitted.max(ui.min_size().x);
        }
    }
}

impl<M: Model> egui_table::TableDelegate for Delegate<'_, M> {
    fn header_cell_ui(&mut self, ui: &mut Ui, cell: &egui_table::HeaderCellInfo) {
        let index = cell.col_range.start;
        self.header_top = Some(ui.max_rect().top());
        if let Some(edge) = self.widths.get(index).and_then(|width| edges::of_title(ui, index, *width)) {
            self.edges.push(edge);
        }
        if index == self.columns.len() {
            // `egui_table` lays out a title in the fixed part and in the part that scrolls
            // to the side. The part that scrolls gives the smaller place.
            let left = ui.max_rect().left();
            self.columns_end = Some(self.columns_end.map_or(left, |end| end.min(left)));
        }
        if cells::is_clipped(ui) {
            return;
        }
        let Some(column) = self.columns.get(index) else {
            cells::filler_header(ui, self.palette);
            return;
        };
        let descending = (self.sort.column == index).then_some(self.sort.descending);
        if cells::header(ui, self.palette, column, descending) {
            self.header_clicked = Some(index);
        }
        self.measure(ui, index);
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
        self.measure(ui, cell.col_nr);
    }

    fn default_row_height(&self) -> f32 {
        ROW_HEIGHT
    }
}

#[cfg(test)]
mod tests;
