//! The widths of the columns of a table. The table owns them, not `egui_table`: they stay
//! through new data, a sort, a filter and a page that hides the table.
//!
//! No column takes the free width. A drag of the edge at the right of a column changes that
//! column only, so the edge stays at the pointer and the columns to its right move with it.
//! A column that took the free width would give back each point that the drag added, and
//! the edge would stay where it was while the drag added more width in each frame. The free
//! width goes to a filler after the last column: the bar of the titles and the rows go on
//! to the edge of the table, and a narrow table scrolls to the side.

use super::model::Column;

/// The narrowest column. A drag stops at this width.
pub(super) const NARROWEST: f32 = 48.0;
/// The widest column.
pub(super) const WIDEST: f32 = 2000.0;
/// The width of the vertical scroll bar of the table. The filler leaves it free.
const SCROLL_BAR: f32 = 12.0;

/// The width of each column of a model, in the sequence of the columns.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Widths(Vec<f32>);

impl Widths {
    /// The width that each column of the model gives.
    pub(super) fn new(columns: &[Column]) -> Self {
        Self(columns.iter().map(|column| column.width.clamp(NARROWEST, WIDEST)).collect())
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = f32> + '_ {
        self.0.iter().copied()
    }

    /// The width of the filler after the last column in a table of this width.
    pub(super) fn filler(&self, available: f32) -> f32 {
        (available - self.0.iter().sum::<f32>() - SCROLL_BAR).max(0.0)
    }

    /// Takes the width of a drag of the user. A column that the model does not have and
    /// a width that is not a number change nothing.
    pub(super) fn set(&mut self, index: usize, width: f32) {
        if let Some(old) = self.0.get_mut(index)
            && width.is_finite()
        {
            *old = width.clamp(NARROWEST, WIDEST);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COLUMNS: [Column; 3] = [Column::new("Name", 200.0), Column::new("Age", 20.0), Column::new("Node", 3000.0)];

    #[test]
    fn each_width_stays_inside_its_limits() {
        let mut widths = Widths::new(&COLUMNS);
        assert_eq!(widths, Widths(vec![200.0, NARROWEST, WIDEST]));
        widths.set(0, 10.0);
        widths.set(1, 75.5);
        widths.set(2, f32::NAN);
        widths.set(3, 100.0);
        assert_eq!(widths, Widths(vec![NARROWEST, 75.5, WIDEST]));
    }

    #[test]
    fn the_filler_takes_the_free_width_and_leaves_the_scroll_bar_free() {
        let widths = Widths(vec![200.0, 100.0]);
        assert_eq!(widths.filler(1000.0), 1000.0 - 300.0 - SCROLL_BAR);
        assert_eq!(widths.filler(300.0), 0.0, "a narrow table scrolls to the side");
    }
}
