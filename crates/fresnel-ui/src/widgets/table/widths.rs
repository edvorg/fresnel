//! The widths of the columns of a table. The table owns them, not `egui_table`: they stay
//! through new data, a sort, a filter and a page that hides the table.
//!
//! No column takes the free width. A drag of the edge at the right of a column changes that
//! column only, so the edge stays at the pointer and the columns to its right move with it.
//! A column that took the free width would give back each point that the drag added, and
//! the edge would stay where it was while the drag added more width in each frame. The free
//! width goes to a filler after the last column: the bar of the titles and the rows go on
//! to the edge of the table, and a narrow table scrolls to the side.
//!
//! The filler also holds the place that a scrolled table showed. At the right end of a
//! narrow table, a narrower column would make the content narrower than that place: the
//! scroll area would then move each column to the right, the edge would stay away from the
//! pointer, and each frame would take more width from the column.
//!
//! The state file keeps the widths that the user changed, in whole points (`TableLayout`).

use fresnel_core::state_file::TableLayout;

use super::model::Column;

/// The narrowest column. A drag stops at this width.
pub(super) const NARROWEST: f32 = 48.0;
/// The widest column.
pub(super) const WIDEST: f32 = 2000.0;
/// The width of the vertical scroll bar of the table. The filler leaves it free.
const SCROLL_BAR: f32 = 12.0;
/// A smaller distance to the side is no scroll: the table and `egui_table` add the widths
/// in a different sequence, so their sums can differ in the last bits.
const NO_SCROLL: f32 = 0.5;

/// The width of each column of a model, in the sequence of the columns, and what the last
/// frame showed.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Widths {
    columns: Vec<f32>,
    shown: Option<Shown>,
    /// True when the user changed a width after the last layout.
    changed: bool,
}

/// What one frame showed of the table.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Shown {
    /// The width of the columns with the filler.
    content: f32,
    /// How far the table was scrolled to the side.
    scrolled: f32,
}

impl Widths {
    /// The width that each column of the model gives.
    pub(super) fn new(columns: &[Column]) -> Self {
        Self { columns: columns.iter().map(width_of).collect(), shown: None, changed: false }
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = f32> + '_ {
        self.columns.iter().copied()
    }

    /// The width of the filler after the last column in a table of this width: the free
    /// width, or the width that keeps the place that the last frame showed of a scrolled
    /// table. That place is never wider than the last content, so a scroll to the right
    /// cannot make the table wider.
    pub(super) fn filler(&self, available: f32) -> f32 {
        let sum = self.sum();
        let free = available - sum - SCROLL_BAR;
        let scrolled = self.shown.filter(|shown| shown.scrolled >= NO_SCROLL);
        let keep = scrolled.map_or(0.0, |shown| shown.content.min(shown.scrolled + available) - sum);
        free.max(keep).max(0.0)
    }

    /// Takes what a frame showed, before a drag of that frame changes a width. `end` is the
    /// place of the end of the columns on the screen, from the left edge of the table, and
    /// `filler` the width of the filler in that frame.
    pub(super) fn shown(&mut self, end: f32, filler: f32) {
        let sum = self.sum();
        self.shown = Some(Shown { content: sum + filler, scrolled: (sum - end).max(0.0) });
    }

    /// Takes the width of a drag of the user. A column that the model does not have and
    /// a width that is not a number change nothing.
    pub(super) fn set(&mut self, index: usize, width: f32) {
        if let Some(old) = self.columns.get_mut(index)
            && width.is_finite()
        {
            let width = width.clamp(NARROWEST, WIDEST);
            self.changed |= *old != width;
            *old = width;
        }
    }

    /// True when the user changed a width after the last layout.
    pub(super) fn changed(&self) -> bool {
        self.changed
    }

    /// The layout after a change of the user: each width that differs from the width of
    /// its column in the model. `None` when no width changed after the last layout.
    pub(super) fn take_layout(&mut self, columns: &[Column]) -> Option<TableLayout> {
        if !std::mem::take(&mut self.changed) {
            return None;
        }
        let differ = columns.iter().zip(&self.columns).filter_map(|(column, width)| {
            let width = points(*width);
            (width != points(width_of(column))).then(|| (column.title.to_owned(), width))
        });
        Some(TableLayout { widths: differ.collect() })
    }

    /// Takes the widths of a saved layout. A title that the model does not have changes
    /// nothing: a later version of the table can have other columns.
    pub(super) fn restore(&mut self, columns: &[Column], layout: &TableLayout) {
        for (column, width) in columns.iter().zip(&mut self.columns) {
            if let Some(saved) = layout.widths.get(column.title) {
                *width = (*saved as f32).clamp(NARROWEST, WIDEST);
            }
        }
    }

    fn sum(&self) -> f32 {
        self.columns.iter().sum()
    }
}

/// The width that a column of the model gives.
fn width_of(column: &Column) -> f32 {
    column.width.clamp(NARROWEST, WIDEST)
}

/// A width in whole points. The limits of a width make the value fit.
fn points(width: f32) -> u32 {
    width.round() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    const COLUMNS: [Column; 3] = [Column::new("Name", 200.0), Column::new("Age", 20.0), Column::new("Node", 3000.0)];

    fn widths(columns: &[f32]) -> Widths {
        Widths { columns: columns.to_vec(), shown: None, changed: false }
    }

    #[test]
    fn each_width_stays_inside_its_limits() {
        let mut widths = Widths::new(&COLUMNS);
        assert_eq!(widths.columns, [200.0, NARROWEST, WIDEST]);
        widths.set(0, 10.0);
        widths.set(1, 75.5);
        widths.set(2, f32::NAN);
        widths.set(3, 100.0);
        assert_eq!(widths.columns, [NARROWEST, 75.5, WIDEST]);
    }

    #[test]
    fn the_filler_takes_the_free_width_and_leaves_the_scroll_bar_free() {
        let mut widths = widths(&[200.0, 100.0]);
        assert_eq!(widths.filler(1000.0), 1000.0 - 300.0 - SCROLL_BAR);
        assert_eq!(widths.filler(300.0), 0.0, "a narrow table scrolls to the side");

        // A table that is not scrolled keeps no place: a narrower window gives less free width.
        widths.shown(300.0, 1000.0 - 300.0 - SCROLL_BAR);
        assert_eq!(widths.filler(800.0), 800.0 - 300.0 - SCROLL_BAR);
        widths.shown(300.0 - NO_SCROLL / 2.0, 0.0);
        assert_eq!(widths.filler(800.0), 800.0 - 300.0 - SCROLL_BAR, "a difference in the last bits");
    }

    #[test]
    fn the_filler_keeps_the_place_that_a_scrolled_table_showed() {
        // 1040 points of columns in a table of 900, at its right end: 140 points to the side.
        let mut widths = widths(&[520.0, 520.0]);
        widths.shown(900.0, 0.0);
        widths.set(1, 510.0);
        assert_eq!(widths.filler(900.0), 10.0, "the content keeps its 1040 points");
        widths.shown(900.0 - 10.0, 10.0);
        assert_eq!(widths.filler(900.0), 10.0, "the next frame keeps the same place");

        // A wider column takes the width of the filler first.
        widths.set(1, 515.0);
        assert_eq!(widths.filler(900.0), 5.0);

        // A scroll to the left makes the place smaller: the user cannot scroll further
        // to the right than the columns go.
        widths.shown(1035.0 - 100.0, 5.0);
        assert_eq!(widths.filler(900.0), 0.0);
        // And a wider window shows more of the columns.
        widths.shown(900.0, 0.0);
        assert_eq!(widths.filler(1000.0), 0.0);
    }

    #[test]
    fn a_layout_has_each_width_that_the_user_changed_and_loads_again() {
        let mut widths = Widths::new(&COLUMNS);
        assert_eq!(widths.take_layout(&COLUMNS), None, "no change");
        widths.set(0, 200.0);
        assert!(!widths.changed(), "the same width is no change");
        widths.set(0, 260.4);
        widths.set(1, 75.5);
        assert!(widths.changed());
        let layout = widths.take_layout(&COLUMNS).expect("a change");
        let expected = [("Age".to_owned(), 76), ("Name".to_owned(), 260)];
        assert_eq!(layout.widths.into_iter().collect::<Vec<_>>(), expected, "in whole points");
        assert_eq!(widths.take_layout(&COLUMNS), None, "one layout for each change");

        // A width back at the width of the model leaves the layout.
        widths.set(1, NARROWEST);
        assert_eq!(widths.take_layout(&COLUMNS).expect("a change").widths.len(), 1);

        let mut saved = Widths::new(&COLUMNS);
        let layout = TableLayout {
            widths: [("Name", 260), ("Node", 9999), ("Gone", 90)]
                .map(|(title, width)| (title.to_owned(), width))
                .into(),
        };
        saved.restore(&COLUMNS, &layout);
        assert_eq!(saved.columns, [260.0, NARROWEST, WIDEST], "a title that the model does not have changes nothing");
        assert!(!saved.changed(), "a saved layout is no change of the user");
    }
}
