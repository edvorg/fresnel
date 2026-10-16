//! The selection of the user and the scroll of the view through the history.

use alacritty_terminal::grid::{Dimensions, Scroll as GridScroll};
use alacritty_terminal::index::{Column, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::viewport_to_point;

use super::Terminal;
use super::screen::Screen;

/// A place on the screen: the cell of a row, and the half of it under the pointer. A place
/// out of the screen counts as its nearest cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spot {
    pub row: usize,
    pub column: usize,
    /// True in the right half of the cell.
    pub right: bool,
}

/// What a selection takes from its start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectKind {
    /// The cells from the start to the end, as text flows.
    Cells,
    /// Whole words: a double click.
    Words,
    /// Whole lines: a triple click.
    Lines,
    /// A rectangle.
    Block,
}

/// A move of the view through the history.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scroll {
    /// Lines up for a positive count.
    Lines(i32),
    PageUp,
    PageDown,
    Bottom,
}

impl Terminal {
    /// Starts a selection at a place.
    pub fn select(&self, kind: SelectKind, at: Spot) {
        let kind = match kind {
            SelectKind::Cells => SelectionType::Simple,
            SelectKind::Words => SelectionType::Semantic,
            SelectKind::Lines => SelectionType::Lines,
            SelectKind::Block => SelectionType::Block,
        };
        let mut screen = self.shared.screen.lock();
        let (point, side) = point(&screen, at);
        screen.term.selection = Some(Selection::new(kind, point, side));
        screen.changed();
    }

    /// Moves the end of the selection to a place.
    pub fn select_to(&self, at: Spot) {
        let mut screen = self.shared.screen.lock();
        let (point, side) = point(&screen, at);
        if let Some(selection) = &mut screen.term.selection {
            selection.update(point, side);
            screen.changed();
        }
    }

    /// Clears the selection. Gives true when there was one.
    pub fn clear_selection(&self) -> bool {
        let mut screen = self.shared.screen.lock();
        let had = screen.term.selection.take().is_some();
        if had {
            screen.changed();
        }
        had
    }

    /// The text of the selection, with a line end after each line that did not wrap.
    pub fn selection_text(&self) -> Option<String> {
        self.shared.screen.lock().term.selection_to_string().filter(|text| !text.is_empty())
    }

    /// Moves the view through the history. The alternate screen has no history.
    pub fn scroll(&self, scroll: Scroll) {
        let scroll = match scroll {
            Scroll::Lines(lines) => GridScroll::Delta(lines),
            Scroll::PageUp => GridScroll::PageUp,
            Scroll::PageDown => GridScroll::PageDown,
            Scroll::Bottom => GridScroll::Bottom,
        };
        let mut screen = self.shared.screen.lock();
        screen.term.scroll_display(scroll);
        screen.changed();
    }

    /// Changes the count of lines of history. Fewer lines drop the oldest ones.
    pub fn set_scrollback(&self, lines: usize) {
        self.shared.screen.lock().set_scrollback(lines);
    }
}

/// The point of the grid at a place of the screen.
fn point(screen: &Screen, at: Spot) -> (Point, Side) {
    let term = &screen.term;
    let column = at.column.min(term.columns().saturating_sub(1));
    let row = at.row.min(term.screen_lines().saturating_sub(1));
    let point = viewport_to_point(term.grid().display_offset(), Point::new(row, Column(column)));
    (point, if at.right { Side::Right } else { Side::Left })
}
