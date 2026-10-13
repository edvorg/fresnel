//! What a kind gives to its table: columns, rows, the sequence of two rows and the cells.

use std::borrow::Cow;
use std::cmp::Ordering;
use std::sync::Arc;

use egui::Color32;
use fresnel_core::cluster::ClusterId;

use super::worker::Cancel;
use crate::widgets::tone::Tone;

/// The identity of one object in the app: its cluster and its UID (design rule 8). The
/// cursor and the selection of a table hold this value and never a row number, so a new
/// sort, a new filter and new data do not move them to some other object.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ObjectId {
    pub(crate) cluster: ClusterId,
    pub(crate) uid: Arc<str>,
}

/// One column of a table.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Column {
    pub(crate) title: &'static str,
    pub(crate) width: f32,
    /// A number aligns to the right.
    pub(crate) numeric: bool,
}

impl Column {
    pub(crate) const fn new(title: &'static str, width: f32) -> Self {
        Self { title, width, numeric: false }
    }

    pub(crate) const fn numeric(mut self) -> Self {
        self.numeric = true;
        self
    }
}

/// The sort of a table. A table always has one, so its rows always have one sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Sort {
    /// The position of the column in the columns of the model.
    pub(crate) column: usize,
    pub(crate) descending: bool,
}

impl Sort {
    /// The sort after a click on the title of `column`: a second click on the sort
    /// column turns the sequence, and a click on some other column sorts by it.
    pub(crate) fn clicked(self, column: usize) -> Self {
        Self { column, descending: self.column == column && !self.descending }
    }
}

/// What stands before the text of a cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Lead<'a> {
    None,
    /// A dot in this color: a state, or the color of a cluster.
    Dot(Color32),
    /// The freshness mark of a row from a store that is not live (design rule 4). The
    /// text tells since when the row is old; the pointer and a screen reader get it.
    Old(&'a str),
}

/// The content of one cell.
#[derive(Clone, Debug)]
pub(crate) struct Cell<'a> {
    pub(crate) text: Cow<'a, str>,
    pub(crate) tone: Tone,
    pub(crate) lead: Lead<'a>,
}

impl<'a> Cell<'a> {
    pub(crate) fn new(text: impl Into<Cow<'a, str>>) -> Self {
        Self { text: text.into(), tone: Tone::Normal, lead: Lead::None }
    }

    pub(crate) fn tone(mut self, tone: Tone) -> Self {
        self.tone = tone;
        self
    }

    pub(crate) fn lead(mut self, lead: Lead<'a>) -> Self {
        self.lead = lead;
        self
    }
}

/// The rows that pass the filter of a view, as the worker thread made them.
pub(crate) struct Computed<Row, Summary> {
    /// The rows, in any sequence. The table sorts them.
    pub(crate) rows: Vec<Row>,
    /// The count of objects before the filter.
    pub(crate) total: usize,
    /// What the view shows of all objects besides the rows.
    pub(crate) summary: Summary,
}

/// What a kind gives to its table. The worker thread of the table calls `rows`,
/// `compare` and `tie`; the UI thread calls the rest.
pub(crate) trait Model: Send + Sync + 'static {
    /// The objects that the rows come from: the snapshots of the stores. A clone copies
    /// pointers only.
    type Data: Clone + Send + 'static;
    /// What selects the rows: the filters of the view.
    type Filter: Clone + PartialEq + Send + 'static;
    /// One row.
    type Row: Send + 'static;
    /// What the view reads of all objects besides the rows.
    type Summary: Send + 'static;
    /// What a cell needs at its paint that is not in its row: the clock, the colors and
    /// the state of each store.
    type Scene;

    fn columns(&self) -> &'static [Column];

    /// The rows that pass the filter. `None` after `cancel` is set: the result has no use.
    fn rows(
        &self,
        data: &Self::Data,
        filter: &Self::Filter,
        cancel: &Cancel<'_>,
    ) -> Option<Computed<Self::Row, Self::Summary>>;

    /// The sequence of two rows by the values of one column. A value comes from the object
    /// alone, with no clock, so a row moves only when its own value changes.
    fn compare(&self, a: &Self::Row, b: &Self::Row, column: usize) -> Ordering;

    /// The fixed last key of each sort: the sequence of two rows with equal values. It
    /// never gives `Equal` for two objects, so the rows have one sequence only.
    fn tie(&self, a: &Self::Row, b: &Self::Row) -> Ordering;

    /// The identity of the object of a row.
    fn id<'a>(&self, row: &'a Self::Row) -> &'a ObjectId;

    fn cell<'a>(&self, row: &'a Self::Row, column: usize, scene: &'a Self::Scene) -> Cell<'a>;
}

/// True when `text` has `needle` in it, with no difference between the cases of ASCII
/// letters. `needle` is in lower case.
pub(crate) fn contains_ignore_case(text: &str, needle: &str) -> bool {
    let (text, needle) = (text.as_bytes(), needle.as_bytes());
    needle.is_empty() || text.windows(needle.len()).any(|window| window.eq_ignore_ascii_case(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_click_on_the_sort_column_turns_the_sequence_and_an_other_column_starts_upward() {
        let by_name = Sort { column: 0, descending: false };
        assert_eq!(by_name.clicked(0), Sort { column: 0, descending: true });
        assert_eq!(by_name.clicked(0).clicked(0), by_name);
        assert_eq!(by_name.clicked(0).clicked(3), Sort { column: 3, descending: false });
    }

    #[test]
    fn the_text_filter_ignores_the_case_of_ascii_letters() {
        assert!(contains_ignore_case("CrashLoopBackOff", "loopback"));
        assert!(contains_ignore_case("anything", ""));
        assert!(!contains_ignore_case("ab", "abc"));
        assert!(!contains_ignore_case("", "a"));
    }
}
