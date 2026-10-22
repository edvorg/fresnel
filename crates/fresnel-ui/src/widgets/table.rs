//! The table of the objects of one kind, on `egui_table` (plan section 11.2). No other
//! module uses that crate, so an upgrade of it changes this file only.
//!
//! `egui_table` gives the virtual rows and the fixed header. This module adds the rows from
//! a worker thread (`widgets/worker`), the sort, the cursor and the selection by the
//! identity of an object, the keys, the widths of the columns and the handles that change them.
//!
//! - `model`: what a kind gives to its table. `rows`: a request and its rows, which the
//!   worker computes. `supply`: the last request and the rows on the screen.
//! - `delegate`: what `egui_table` asks in a frame. `cells`: the paint of each part.
//! - `keys`: the keys. `widths`: the widths of the columns. `choice`: the columns that show.
//! - `layout`: the widths and the columns that the state file keeps.
//! - `edges`: the handle at the right of each column, for a drag and a double click.
//! - `steady`: the place of the rows under the pointer when a part above the table goes away.
//! - `selection`: the selected objects. `select`: what a click and a key do to them.

pub(crate) mod cells;
mod choice;
mod delegate;
mod edges;
mod keys;
mod layout;
pub(crate) mod model;
mod rows;
mod select;
mod selection;
mod steady;
mod supply;
mod widths;

use std::sync::Arc;
use std::time::Duration;

use egui::{EventFilter, Sense, Ui};
use fresnel_core::net::task::Tasks;
use fresnel_core::state_file::TableLayout;

use self::choice::Choice;
use self::delegate::Delegate;
use self::edges::{Edges, Seen};
use self::model::{Model, ObjectId};
pub(crate) use self::rows::Rows;
use self::select::{click, forget_gone, key, position};
pub(crate) use self::selection::Selection;
use self::steady::Steady;
pub(crate) use self::supply::Progress;
use self::supply::Supply;
use self::widths::{FrameWidths, NARROWEST, WIDEST, Widths};
use crate::theme::{self, ROW_HEIGHT};
use crate::widgets::worker::{RequestId, Wake};

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
    choice: Choice,
    edges: Edges,
    /// The layout that the core has of this table, as far as the table knows. `None` before
    /// the core read the state file.
    saved: Option<TableLayout>,
    /// The last answer of `hides`: the rows it read, the object and the answer. A scan of
    /// 10,000 rows in each frame would take a part of each frame.
    hidden: Option<(RequestId, ObjectId, bool)>,
    /// The height of a row: a setting of the user.
    row_height: f32,
    /// True when the table takes the focus of the keys in the next frame.
    focus: bool,
    /// The place of the table in the last frame, which stays while the pointer is over it.
    steady: Steady,
}

impl<M: Model> Table<M> {
    /// Starts the worker thread of the table. `wake` asks for a repaint for a result.
    pub(crate) fn new(tasks: &Tasks, wake: Wake, name: &'static str, model: M) -> Self {
        let model = Arc::new(model);
        let supply = Supply::start(tasks, wake, name, Arc::clone(&model));
        let (widths, choice) = (Widths::new(model.columns()), Choice::new(model.columns()));
        let (selection, edges, steady) = (Selection::default(), Edges::default(), Steady::default());
        let (saved, hidden, reveal, row_height, focus) = (None, None, None, ROW_HEIGHT, false);
        Self { name, model, supply, selection, reveal, widths, choice, edges, saved, hidden, row_height, focus, steady }
    }

    /// Tells the table what its rows come from in this frame, and takes a result of the
    /// worker. `data_serial` grows when the objects change, and `data` gives them.
    pub(crate) fn request(&mut self, filter: &M::Filter, data_serial: u64, data: impl FnOnce() -> M::Data) {
        self.supply.request(filter, data_serial, data);
    }

    /// Sets the height of a row, from the settings of this frame.
    pub(crate) fn set_row_height(&mut self, height: f32) {
        self.row_height = height;
    }

    /// Gives the table the focus of the keys in the next frame that shows rows.
    pub(crate) fn focus(&mut self) {
        self.focus = true;
    }

    pub(crate) fn model(&self) -> &M {
        &self.model
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

    /// Selects this object only, and the next frame scrolls to its row: a link opened it.
    pub(crate) fn select(&mut self, id: ObjectId) {
        self.selection.only(id.clone());
        self.reveal = Some(id);
    }

    /// Gives up the rows, the objects of the last request and the selection. A view
    /// calls this in each frame that does not paint it: it must hold no rows then (plan
    /// section 7.3), and no frame would remove an object that goes away from the selection.
    pub(crate) fn rest(&mut self) {
        self.supply.rest();
        self.selection.clear();
        self.reveal = None;
        self.edges.rest();
        self.steady.rest();
    }

    /// The row of the cursor, if the rows on the screen have its object.
    pub(crate) fn cursor_row(&self) -> Option<&M::Row> {
        let (rows, cursor) = (&self.rows()?.rows, self.selection.cursor()?);
        rows.iter().find(|row| self.model.id(row) == cursor)
    }

    /// True when the rows on the screen are the rows of the last request and none of them
    /// is the row of this object: a filter hides it.
    pub(crate) fn hides(&mut self, id: &ObjectId) -> bool {
        let Some(rows) = self.supply.shown_id().filter(|_| self.supply.is_current()) else {
            return false;
        };
        match &self.hidden {
            Some((seen, object, hidden)) if *seen == rows && object == id => *hidden,
            _ => {
                let hidden = self.rows().is_some_and(|shown| position(&*self.model, &shown.rows, id).is_none());
                self.hidden = Some((rows, id.clone(), hidden));
                hidden
            }
        }
    }

    /// Draws the table into the rest of `ui`. `empty` tells why the table has no row, and
    /// `menu` draws the menu of the row of an object. Gives what the user did to the cursor.
    pub(crate) fn show(
        &mut self,
        ui: &mut Ui,
        scene: &M::Scene,
        empty: impl FnOnce() -> String,
        menu: &mut dyn FnMut(&mut Ui, &ObjectId),
    ) -> Option<Act> {
        self.steady.hold(ui);
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
        let (model, columns, visible) = (&*self.model, self.model.columns(), self.choice.visible());
        let rect = ui.available_rect_before_wrap();
        // The focus of the keys is an area below the rows. The rows are above it and
        // get the clicks.
        let focus_id = ui.make_persistent_id((self.name, "keys"));
        let focus = ui.interact(rect, focus_id, Sense::focusable_noninteractive());
        if std::mem::take(&mut self.focus) {
            ui.memory_mut(|memory| memory.request_focus(focus_id));
        }

        // The vertical scroll bar takes its place at the right of the rows.
        let place = rect.width() - ui.spacing().scroll.allocated_width();
        if let Some((column, width)) = self.edges.dragged(ui) {
            self.widths.drag(column, width, place);
        }
        let fit = self.edges.take_fit();
        let frame = self.widths.for_frame(place, &visible);
        let table_columns = self.table_columns(&visible, &frame, fit);
        let mut table = egui_table::Table::new()
            .id_salt(self.name)
            .num_rows(rows.len() as u64)
            .columns(table_columns.clone())
            .num_sticky_cols(1)
            .headers([egui_table::HeaderRow::new(self.row_height + 2.0)])
            .auto_size_mode(egui_table::AutoSizeMode::Never);
        // The object of a link can wait for the rows of a new filter. It stays until the
        // rows of the last request are on the screen.
        let reveal = self.reveal.as_ref().and_then(|target| position(model, rows, target));
        if reveal.is_some() || self.supply.is_current() {
            self.reveal = None;
        }
        if let Some(row) = reveal {
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
            visible: &visible,
            sort: self.supply.sort(),
            selection: &self.selection,
            widths: &frame.columns,
            header_clicked: None,
            row_clicked: None,
            columns_end: None,
            seen: Seen::default(),
            fit,
            fitted: 0.0,
            menu,
            row_height: self.row_height,
        };
        table.show(ui, &mut delegate);
        let (header_clicked, row_clicked) = (delegate.header_clicked, delegate.row_clicked);
        if let Some(end) = delegate.columns_end {
            self.widths.shown(&frame, end - rect.left());
        }
        if let Some(column) = fit.filter(|_| delegate.fitted > 0.0) {
            self.widths.set(column, delegate.fitted);
        }
        let id = |column: usize| ui.make_persistent_id((self.name, columns.get(column).map(|c| c.title), "edge"));
        let widths = |column: usize| {
            let position = visible.iter().position(|index| *index == column)?;
            Some((*frame.columns.get(position)?, self.widths.get(column)?))
        };
        self.edges.handles(ui, &delegate.seen, rect, id, widths);

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
            let page_rows = (rect.height() / self.row_height).floor() as usize;
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

    /// The columns of `egui_table`: one for each column that shows, with the widths of
    /// this frame, and the filler after them. The table changes a width, not `egui_table`:
    /// it measures the cells of the column `fit` only.
    fn table_columns(&self, visible: &[usize], frame: &FrameWidths, fit: Option<usize>) -> Vec<egui_table::Column> {
        let columns = self.model.columns();
        let shown = visible.iter().filter_map(|index| Some((*index, columns.get(*index)?)));
        let columns = shown.zip(&frame.columns).map(|((index, column), width)| {
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

impl<M: Model> egui_table::TableDelegate for Delegate<'_, M> {
    fn header_cell_ui(&mut self, ui: &mut Ui, cell: &egui_table::HeaderCellInfo) {
        self.header(ui, cell.col_range.start);
    }

    fn row_ui(&mut self, ui: &mut Ui, row_nr: u64) {
        self.row_background(ui, row_nr);
    }

    fn cell_ui(&mut self, ui: &mut Ui, cell: &egui_table::CellInfo) {
        self.cell(ui, cell.row_nr, cell.col_nr);
    }

    fn default_row_height(&self) -> f32 {
        self.row_height
    }
}

#[cfg(test)]
mod tests;
