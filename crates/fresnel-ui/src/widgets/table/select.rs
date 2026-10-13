//! What a click and a key do to the selection of a table. The selection holds objects,
//! so each function finds the rows of its objects in the rows on the screen.

use egui::Modifiers;

use super::Act;
use super::keys::{self, Nav};
use super::model::{Model, ObjectId};
use super::selection::Selection;

/// The row of an object. The call reads each row to it, so it runs for a click or a key
/// and not for each frame.
pub(super) fn position<M: Model>(model: &M, rows: &[M::Row], id: &ObjectId) -> Option<usize> {
    rows.iter().position(|row| model.id(row) == id)
}

pub(super) fn click<M: Model>(model: &M, rows: &[M::Row], selection: &mut Selection, row: usize, modifiers: Modifiers) {
    let Some(target) = rows.get(row).map(|row| model.id(row).clone()) else {
        return;
    };
    if modifiers.shift {
        select_range(model, rows, selection, row, target);
    } else if modifiers.command {
        selection.toggle(target);
    } else {
        selection.only(target);
    }
}

/// Selects the rows from the anchor to `row`. An anchor that the rows do not have, for
/// example behind a filter, gives a range of this one row with a new anchor.
fn select_range<M: Model>(model: &M, rows: &[M::Row], selection: &mut Selection, row: usize, target: ObjectId) {
    let anchor = selection.anchor().and_then(|anchor| Some((position(model, rows, anchor)?, anchor.clone())));
    let (from, anchor) = anchor.unwrap_or_else(|| (row, target.clone()));
    let range = rows.get(from.min(row)..=from.max(row)).unwrap_or_default();
    selection.range(range.iter().map(|row| model.id(row).clone()), anchor, target);
}

/// Applies a key to the selection. Gives what the key did to the cursor.
pub(super) fn key<M: Model>(
    model: &M,
    rows: &[M::Row],
    selection: &mut Selection,
    nav: Nav,
    extend: bool,
) -> Option<Act> {
    match nav {
        Nav::SelectAll => {
            selection.all(rows.iter().map(|row| model.id(row).clone()));
            None
        }
        Nav::Clear => {
            selection.clear();
            Some(Act::Cleared)
        }
        Nav::By(_) | Nav::First | Nav::Last => {
            let cursor = selection.cursor().and_then(|cursor| position(model, rows, cursor));
            let row = keys::target(nav, cursor, rows.len())?;
            let target = model.id(rows.get(row)?).clone();
            if extend {
                select_range(model, rows, selection, row, target);
            } else {
                selection.only(target);
            }
            Some(Act::Cursor)
        }
    }
}
