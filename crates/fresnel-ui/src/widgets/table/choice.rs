//! Which columns of a table show (plan section 11.2). The model gives each column its
//! first choice. The user shows or hides a column, and the state file keeps each choice
//! that differs from the one of the model, by the title of the column.

use std::collections::BTreeMap;

use fresnel_core::state_file::TableLayout;

use super::model::Column;

/// The choice of each column of a model, in the sequence of the columns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Choice {
    shown: Vec<bool>,
    /// True when the user changed a choice after the last layout.
    changed: bool,
}

impl Choice {
    /// The first choice of each column of the model.
    pub(super) fn new(columns: &[Column]) -> Self {
        Self { shown: columns.iter().map(|column| column.shown).collect(), changed: false }
    }

    /// The positions of the columns that show, in their sequence. The first column always
    /// shows: it has the name and the freshness mark of each row.
    pub(super) fn visible(&self) -> Vec<usize> {
        (0..self.shown.len()).filter(|index| *index == 0 || self.is_shown(*index)).collect()
    }

    pub(super) fn is_shown(&self, index: usize) -> bool {
        index == 0 || self.shown.get(index).copied().unwrap_or(false)
    }

    /// Shows or hides a column for the user. The first column and a column that the model
    /// does not have change nothing.
    pub(super) fn set(&mut self, index: usize, shown: bool) {
        if let Some(old) = self.shown.get_mut(index).filter(|_| index > 0)
            && *old != shown
        {
            *old = shown;
            self.changed = true;
        }
    }

    /// True when the user changed a choice after the last layout.
    pub(super) fn changed(&self) -> bool {
        self.changed
    }

    /// Each choice that differs from the first choice of its column, after a change of the
    /// user. `None` when no choice changed after the last layout.
    pub(super) fn take_layout(&mut self, columns: &[Column]) -> Option<BTreeMap<String, bool>> {
        if !std::mem::take(&mut self.changed) {
            return None;
        }
        let differ = columns.iter().zip(&self.shown).filter(|(column, shown)| column.shown != **shown);
        Some(differ.map(|(column, shown)| (column.title.to_owned(), *shown)).collect())
    }

    /// Takes the choices of a saved layout. A title that the model does not have changes
    /// nothing: a later version of the table can have other columns.
    pub(super) fn restore(&mut self, columns: &[Column], layout: &TableLayout) {
        for (column, shown) in columns.iter().zip(&mut self.shown) {
            if let Some(saved) = layout.columns.get(column.title) {
                *shown = *saved;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COLUMNS: [Column; 4] = [
        Column::new("Name", 200.0),
        Column::new("Node", 100.0),
        Column::new("Labels", 100.0).hidden(),
        Column::new("Age", 60.0),
    ];

    #[test]
    fn the_first_column_always_shows_and_a_hidden_column_shows_after_a_choice_of_the_user() {
        let mut choice = Choice::new(&COLUMNS);
        assert_eq!(choice.visible(), [0, 1, 3]);
        choice.set(0, false);
        choice.set(9, true);
        assert!(!choice.changed(), "no change");
        choice.set(2, true);
        choice.set(1, false);
        assert_eq!(choice.visible(), [0, 2, 3]);
        let layout = choice.take_layout(&COLUMNS).expect("a change");
        assert_eq!(layout, BTreeMap::from([("Labels".to_owned(), true), ("Node".to_owned(), false)]));
        assert_eq!(choice.take_layout(&COLUMNS), None, "one layout for each change");

        // A choice back at the first one leaves the layout.
        choice.set(1, true);
        assert_eq!(choice.take_layout(&COLUMNS), Some(BTreeMap::from([("Labels".to_owned(), true)])));
    }

    #[test]
    fn a_saved_layout_restores_the_choices_and_is_no_change_of_the_user() {
        let mut choice = Choice::new(&COLUMNS);
        let columns = [("Labels", true), ("Age", false), ("Gone", true)];
        let layout = TableLayout {
            columns: columns.map(|(title, shown)| (title.to_owned(), shown)).into(),
            ..TableLayout::default()
        };
        choice.restore(&COLUMNS, &layout);
        assert_eq!((choice.visible(), choice.changed()), (vec![0, 1, 2], false));
    }
}
