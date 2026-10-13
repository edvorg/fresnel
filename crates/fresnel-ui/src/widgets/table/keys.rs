//! The keys of a table. The table reads them only while it has the focus.

use egui::{Key, Modifiers, Ui};

/// What a key asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Nav {
    /// Moves the cursor by this count of rows.
    By(i64),
    First,
    Last,
    SelectAll,
    Clear,
}

/// Reads one key from the input. The second value is true when Shift makes a range.
pub(super) fn read(ui: &Ui, page_rows: usize) -> Option<(Nav, bool)> {
    let page = i64::try_from(page_rows.max(1)).unwrap_or(i64::MAX);
    let moves = [
        (Key::ArrowDown, Nav::By(1)),
        (Key::ArrowUp, Nav::By(-1)),
        (Key::PageDown, Nav::By(page)),
        (Key::PageUp, Nav::By(-page)),
        (Key::Home, Nav::First),
        (Key::End, Nav::Last),
    ];
    ui.input_mut(|input| {
        // Shift comes first: a pattern with no Shift takes a key with Shift too.
        for extend in [true, false] {
            let modifiers = if extend { Modifiers::SHIFT } else { Modifiers::NONE };
            if let Some((_, nav)) = moves.iter().find(|(key, _)| input.consume_key(modifiers, *key)) {
                return Some((*nav, extend));
            }
        }
        if input.consume_key(Modifiers::COMMAND, Key::A) {
            return Some((Nav::SelectAll, false));
        }
        input.consume_key(Modifiers::NONE, Key::Escape).then_some((Nav::Clear, false))
    })
}

/// The row that a key moves the cursor to. `cursor` is the row of the cursor, if the
/// rows have its object. `None` for a table with no row and for a key that moves nothing.
pub(super) fn target(nav: Nav, cursor: Option<usize>, rows: usize) -> Option<usize> {
    let last = rows.checked_sub(1)?;
    match nav {
        // With no cursor, the first key selects the first row.
        Nav::By(_) if cursor.is_none() => Some(0),
        Nav::By(rows) => {
            let from = i64::try_from(cursor.unwrap_or_default()).unwrap_or(i64::MAX);
            let to = from.saturating_add(rows).clamp(0, i64::try_from(last).unwrap_or(i64::MAX));
            usize::try_from(to).ok()
        }
        Nav::First => Some(0),
        Nav::Last => Some(last),
        Nav::SelectAll | Nav::Clear => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cursor_stays_in_the_rows_and_the_first_key_selects_the_first_row() {
        assert_eq!(target(Nav::By(1), None, 10), Some(0));
        assert_eq!(target(Nav::By(-1), None, 10), Some(0));
        assert_eq!(target(Nav::By(1), Some(3), 10), Some(4));
        assert_eq!(target(Nav::By(5), Some(8), 10), Some(9));
        assert_eq!(target(Nav::By(-5), Some(2), 10), Some(0));
        assert_eq!(target(Nav::By(i64::MAX), Some(2), 10), Some(9));
        assert_eq!((target(Nav::First, Some(2), 10), target(Nav::Last, Some(2), 10)), (Some(0), Some(9)));
        assert_eq!(target(Nav::By(1), Some(0), 0), None, "a table with no row");
        assert_eq!((target(Nav::SelectAll, Some(1), 10), target(Nav::Clear, Some(1), 10)), (None, None));
    }
}
