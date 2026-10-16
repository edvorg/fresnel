//! The list of the marks of the check below the text: each error and hint with its line and
//! column. A click on one, or F8 and Shift+F8 in the text, goes to its place.

use egui::{Label, RichText, ScrollArea, Sense, Ui};
use fresnel_core::manifest::check::{Check, Level, Mark};

use super::text::{Pos, TextArea};
use crate::theme::{self, Palette};
use crate::widgets::dot;

/// The most rows of the list that show at once; it scrolls for more.
const ROWS: usize = 4;

/// The height of one row of the list.
fn row(ui: &Ui) -> f32 {
    ui.spacing().interact_size.y + ui.spacing().item_spacing.y
}

/// The height of the list of a check: its rows, at most `ROWS`.
pub(super) fn height(ui: &Ui, check: &Check) -> f32 {
    let rows = check.marks.len() + usize::from(check.more > 0);
    rows.min(ROWS) as f32 * row(ui)
}

/// Draws the list. Gives the mark that the user clicked.
pub(super) fn show<'a>(ui: &mut Ui, check: &'a Check) -> Option<&'a Mark> {
    let palette = theme::palette(ui);
    let mut chosen = None;
    // A scroll area is 64 points high at least, more than one row of the list.
    let area = ScrollArea::vertical().id_salt("editor marks").auto_shrink(false).min_scrolled_height(0.0);
    area.show(ui, |ui| {
        for mark in &check.marks {
            ui.horizontal(|ui| {
                dot(ui, color(mark.level, palette));
                let words = format!("Line {}, column {}: {}", mark.line, mark.column, mark.text);
                let response = ui.add(Label::new(RichText::new(words).color(palette.text)).sense(Sense::click()));
                if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    chosen = Some(mark);
                }
            });
        }
        if check.more > 0 {
            let more = format!("and {} more", crate::format::thousands(check.more as u64));
            ui.label(RichText::new(more).color(palette.text_muted));
        }
    });
    chosen
}

/// The words of the result of a check for the bar: "No error", "1 error, 3 hints".
pub(super) fn summary(check: &Check) -> String {
    let count = |level| check.marks.iter().filter(|mark| mark.level == level).count() as u64;
    let errors = count(Level::Error);
    let hints = count(Level::Hint) + check.more as u64;
    match (errors, hints) {
        (0, 0) => "No error".to_owned(),
        (0, _) => format!("No error, {}", crate::format::count(hints, "hint")),
        (_, 0) => crate::format::count(errors, "error"),
        _ => format!("{}, {}", crate::format::count(errors, "error"), crate::format::count(hints, "hint")),
    }
}

/// Selects the place of a mark in the text.
pub(super) fn go_to(text: &mut TextArea, mark: &Mark) {
    let line = mark.line.saturating_sub(1);
    let byte = text
        .line(line)
        .char_indices()
        .nth(mark.column.saturating_sub(1))
        .map_or(text.line(line).len(), |(byte, _)| byte);
    text.select(Pos::new(line, byte)..Pos::new(line, byte));
    text.focus();
}

/// The next mark after the cursor, or the one before it, round the end.
pub(super) fn next(check: &Check, cursor: Pos, forward: bool) -> Option<&Mark> {
    let line = |mark: &Mark| mark.line.saturating_sub(1);
    let found = if forward {
        check.marks.iter().find(|mark| line(mark) > cursor.line)
    } else {
        check.marks.iter().rev().find(|mark| line(mark) < cursor.line)
    };
    found.or(if forward { check.marks.first() } else { check.marks.last() })
}

fn color(level: Level, palette: &Palette) -> egui::Color32 {
    match level {
        Level::Error => palette.error,
        Level::Hint => palette.warn,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mark(line: usize, level: Level) -> Mark {
        Mark { line, column: 3, level, text: String::new() }
    }

    #[test]
    fn the_summary_counts_and_the_next_mark_goes_by_line_round_the_end() {
        let check = Check { marks: vec![mark(2, Level::Hint), mark(5, Level::Error), mark(9, Level::Hint)], more: 1 };
        assert_eq!(summary(&check), "1 error, 3 hints");
        assert_eq!(summary(&Check::default()), "No error");
        let line = |found: Option<&Mark>| found.map(|mark| mark.line);
        assert_eq!(line(next(&check, Pos::new(1, 0), true)), Some(5), "the next line, past the mark of the cursor");
        assert_eq!(line(next(&check, Pos::new(8, 0), true)), Some(2), "round the end");
        assert_eq!(line(next(&check, Pos::new(4, 0), false)), Some(2));
        assert_eq!(line(next(&check, Pos::new(0, 0), false)), Some(9), "round the start");
    }
}
