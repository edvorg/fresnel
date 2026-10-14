//! The layout of a table that the state file keeps: the widths and the columns that the
//! user changed. The table reads the saved layout one time and gives back each change.

use std::collections::BTreeMap;

use egui::Ui;
use fresnel_core::app::Command;
use fresnel_core::state_file::TableLayout;

use super::Table;
use super::model::Model;

impl<M: Model> Table<M> {
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
        if !self.choice.changed() {
            self.choice.restore(self.model.columns(), &layout);
        }
        self.saved = Some(layout);
    }

    /// The command that saves the widths and the columns that the user changed. It comes
    /// one time, after the end of the drag, and only after `restore`: the core does not
    /// write the file before it read it.
    pub(crate) fn save_layout(&mut self, ui: &Ui) -> Option<Command> {
        let saved = self.saved.as_mut()?;
        if ui.input(|input| input.pointer.any_down()) {
            return None;
        }
        let columns = self.model.columns();
        let (widths, choice) = (self.widths.take_layout(columns), self.choice.take_layout(columns));
        if widths.is_none() && choice.is_none() {
            return None;
        }
        let layout = TableLayout {
            widths: widths.map_or_else(|| saved.widths.clone(), |widths| widths.widths),
            columns: choice.unwrap_or_else(|| saved.columns.clone()),
        };
        if layout == *saved {
            return None;
        }
        saved.clone_from(&layout);
        Some(Command::SaveTable { table: self.name.to_owned(), layout })
    }

    /// The title, the choice and the place of each column, for the menu of the columns.
    /// The first column always shows.
    pub(crate) fn column_choices(&self) -> impl Iterator<Item = (usize, &'static str, bool)> + '_ {
        let columns = self.model.columns().iter().enumerate();
        columns.skip(1).map(|(index, column)| (index, column.title, self.choice.is_shown(index)))
    }

    /// Shows or hides a column for the user.
    pub(crate) fn show_column(&mut self, index: usize, shown: bool) {
        self.choice.set(index, shown);
    }
}
