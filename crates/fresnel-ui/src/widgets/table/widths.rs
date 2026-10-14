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
//! The first column does not scroll to the side. Wider than the table, it would hide its
//! own edge and each other column, so a frame shows it inside the table. That is no change
//! of its width: a wider window shows the width of the user again.
//!
//! The state file keeps the widths that the user changed, in whole points (`TableLayout`).

use fresnel_core::state_file::TableLayout;

use super::model::Column;

/// The narrowest column. A drag stops at this width.
pub(super) const NARROWEST: f32 = 48.0;
/// The widest column.
pub(super) const WIDEST: f32 = 2000.0;
/// The place that the first column leaves for the columns that scroll to the side.
const SCROLLED_PLACE: f32 = NARROWEST;
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

/// The widths that one frame lays out.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct FrameWidths {
    /// The width of each column, in the sequence of the columns.
    pub(super) columns: Vec<f32>,
    /// The widest that the first column can be in this frame.
    pub(super) first_widest: f32,
    /// The width of the filler after the last column.
    pub(super) filler: f32,
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

    /// The widths of a frame whose columns have the width `place` on the screen: the table
    /// less its vertical scroll bar. The first column stays inside that place, and the
    /// filler comes after the last column.
    pub(super) fn for_frame(&self, place: f32) -> FrameWidths {
        let first_widest = (place - SCROLLED_PLACE).clamp(NARROWEST, WIDEST);
        let mut columns = self.columns.clone();
        if let Some(first) = columns.first_mut() {
            *first = first.min(first_widest);
        }
        let filler = self.filler(place, columns.iter().sum());
        FrameWidths { columns, first_widest, filler }
    }

    /// The width of the filler after columns of the width `sum` in this place: the free
    /// width, or the width that keeps the place that the last frame showed of a scrolled
    /// table. That place is never wider than the last content, so a scroll to the right
    /// cannot make the table wider.
    fn filler(&self, place: f32, sum: f32) -> f32 {
        let free = place - sum;
        let scrolled = self.shown.filter(|shown| shown.scrolled >= NO_SCROLL);
        let keep = scrolled.map_or(0.0, |shown| shown.content.min(shown.scrolled + place) - sum);
        free.max(keep).max(0.0)
    }

    /// Takes what a frame with these widths showed. `end` is the place of the end of the
    /// columns on the screen, from the left edge of the table.
    pub(super) fn shown(&mut self, frame: &FrameWidths, end: f32) {
        let sum: f32 = frame.columns.iter().sum();
        self.shown = Some(Shown { content: sum + frame.filler, scrolled: (sum - end).max(0.0) });
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

    /// One frame in this place, whose columns end at `end`.
    fn show(widths: &mut Widths, place: f32, end: f32) {
        let frame = widths.for_frame(place);
        widths.shown(&frame, end);
    }

    #[test]
    fn the_filler_takes_the_free_width() {
        let mut widths = widths(&[200.0, 100.0]);
        assert_eq!(widths.for_frame(1000.0).filler, 1000.0 - 300.0);
        assert_eq!(widths.for_frame(300.0).filler, 0.0, "a narrow table scrolls to the side");

        // A table that is not scrolled keeps no place: a narrower window gives less free width.
        show(&mut widths, 1000.0, 300.0);
        assert_eq!(widths.for_frame(800.0).filler, 800.0 - 300.0);
        show(&mut widths, 300.0, 300.0 - NO_SCROLL / 2.0);
        assert_eq!(widths.for_frame(800.0).filler, 800.0 - 300.0, "a difference in the last bits");
    }

    #[test]
    fn the_filler_keeps_the_place_that_a_scrolled_table_showed() {
        // 1040 points of columns in a table of 900, at its right end: 140 points to the side.
        let mut widths = widths(&[520.0, 520.0]);
        show(&mut widths, 900.0, 900.0);
        widths.set(1, 510.0);
        assert_eq!(widths.for_frame(900.0).filler, 10.0, "the content keeps its 1040 points");
        show(&mut widths, 900.0, 900.0 - 10.0);
        assert_eq!(widths.for_frame(900.0).filler, 10.0, "the next frame keeps the same place");

        // A wider column takes the width of the filler first.
        widths.set(1, 515.0);
        assert_eq!(widths.for_frame(900.0).filler, 5.0);

        // A scroll to the left makes the place smaller: the user cannot scroll further
        // to the right than the columns go.
        show(&mut widths, 900.0, 1035.0 - 100.0);
        assert_eq!(widths.for_frame(900.0).filler, 0.0);
        // And a wider window shows more of the columns.
        show(&mut widths, 900.0, 900.0);
        assert_eq!(widths.for_frame(1000.0).filler, 0.0);
    }

    #[test]
    fn the_first_column_stays_inside_the_table_and_keeps_the_width_of_the_user() {
        let mut widths = widths(&[2000.0, 100.0]);
        let narrow = widths.for_frame(900.0);
        let first_widest = 900.0 - SCROLLED_PLACE;
        assert_eq!((narrow.columns.as_slice(), narrow.first_widest), ([first_widest, 100.0].as_slice(), first_widest));
        assert_eq!(narrow.filler, 0.0, "the columns fill the table");
        assert_eq!(widths.for_frame(2400.0).columns, [2000.0, 100.0], "a wide table shows the width of the user");
        assert_eq!(widths.for_frame(50.0).columns, [NARROWEST, 100.0], "a table narrower than one column");
        assert_eq!(widths.for_frame(f32::MAX).first_widest, WIDEST);

        // The place that a scrolled table showed counts the widths on the screen: 952 points.
        show(&mut widths, 900.0, 848.0);
        assert_eq!(widths.shown, Some(Shown { content: 952.0, scrolled: 104.0 }));
        assert!(!widths.changed(), "a narrow table is no change of the user");
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
