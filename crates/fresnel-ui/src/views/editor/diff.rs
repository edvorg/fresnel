//! The diff of two texts of an editor (plan section 12.9): the lines that a save changes,
//! with their numbers and three lines of context, and a count of the lines between.
//!
//! The rows are computed one time, for the dialog (`differ`), and the list has virtual rows:
//! the diff of a new text of 8,000 lines lays out the rows on the screen only. A row of a
//! value of a Secret has its text with the value hidden too (`hidden`).

use egui::{RichText, ScrollArea, Ui};
use fresnel_core::manifest::diff;

use super::hidden::hidden;
use crate::format::{count, short};
use crate::theme::{self, Palette};

/// The lines of context before and after each change.
const CONTEXT: usize = 3;
/// The most characters of a row that the dialog lays out.
const ROW_LIMIT: usize = 400;

/// What a row of the diff is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mark {
    /// A line of both texts.
    Same,
    /// A line of the old text only.
    Removed,
    /// A line of the new text only.
    Added,
    /// Lines with no change that do not show; the text gives their count.
    Skipped,
}

/// One row of a diff: its mark, its numbers in the old and the new text from 1, its text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Row {
    pub(super) mark: Mark,
    pub(super) old: Option<usize>,
    pub(super) new: Option<usize>,
    pub(super) text: String,
    /// The text with its value hidden, for a row of a value of a Secret.
    pub(super) hidden: Option<String>,
}

/// The lines of one text, with the hidden text of each line of a value of a Secret.
struct Side<'a> {
    lines: Vec<&'a str>,
    hidden: Vec<Option<String>>,
}

impl<'a> Side<'a> {
    fn new(text: &'a str, secret: bool) -> Self {
        let lines = diff::lines(text);
        let bodies: Vec<&str> = lines.iter().map(|line| line.strip_suffix('\n').unwrap_or(line)).collect();
        let hidden = if secret { hidden(&bodies) } else { Vec::new() };
        Self { lines, hidden }
    }
}

/// The rows of the diff of two texts, of a Secret when `secret` is true. Equal texts give none.
pub(super) fn rows(old: &str, new: &str, secret: bool) -> Vec<Row> {
    let (old, new) = (Side::new(old, secret), Side::new(new, secret));
    let hunks = diff::hunks(&old.lines, &new.lines);
    let mut rows = Vec::new();
    // The next line of the old text that no row shows yet.
    let mut at_old = 0;
    for (index, hunk) in hunks.iter().enumerate() {
        let start = hunk.old.start.saturating_sub(CONTEXT).max(at_old);
        skipped(&mut rows, start - at_old);
        let shift = |line: usize| line + hunk.new.start - hunk.old.start;
        for line in start..hunk.old.start {
            rows.push(row(Mark::Same, (Some(line), Some(shift(line))), &old, line));
        }
        rows.extend(hunk.old.clone().map(|line| row(Mark::Removed, (Some(line), None), &old, line)));
        rows.extend(hunk.new.clone().map(|line| row(Mark::Added, (None, Some(line)), &new, line)));
        let next = hunks.get(index + 1).map_or(old.lines.len(), |next| next.old.start.saturating_sub(CONTEXT));
        let end = (hunk.old.end + CONTEXT).min(next.max(hunk.old.end)).min(old.lines.len());
        let after = |line: usize| line + hunk.new.end - hunk.old.end;
        for line in hunk.old.end..end {
            rows.push(row(Mark::Same, (Some(line), Some(after(line))), &old, line));
        }
        at_old = end;
    }
    if !hunks.is_empty() {
        skipped(&mut rows, old.lines.len() - at_old);
    }
    rows
}

/// A row of a line. A last line with no line feed says so: the save sends that change too.
fn row(mark: Mark, (old, new): (Option<usize>, Option<usize>), side: &Side<'_>, line: usize) -> Row {
    let text = side.lines.get(line).copied().unwrap_or_default();
    let end = |body: &str| {
        if text.ends_with('\n') { body.to_owned() } else { format!("{body}  (no line feed at the end)") }
    };
    let hidden = side.hidden.get(line).and_then(Option::as_deref).map(end);
    let text = end(text.strip_suffix('\n').unwrap_or(text));
    Row { mark, old: old.map(|line| line + 1), new: new.map(|line| line + 1), text, hidden }
}

fn skipped(rows: &mut Vec<Row>, lines: usize) {
    if lines > 0 {
        let text = format!("{} with no change", count(lines as u64, "line"));
        rows.push(Row { mark: Mark::Skipped, old: None, new: None, text, hidden: None });
    }
}

/// Draws the rows in a list of at most `height`, with the numbers of each line. With `hide`,
/// a row of a value of a Secret shows its hidden text.
pub(super) fn show(ui: &mut Ui, rows: &[Row], height: f32, hide: bool) {
    let palette = theme::palette(ui);
    let row_height = ui.text_style_height(&egui::TextStyle::Monospace) + 2.0;
    let area = ScrollArea::both().max_height(height).auto_shrink([false, true]).id_salt("diff rows");
    area.show_rows(ui, row_height, rows.len(), |ui, range| {
        for row in rows.get(range).unwrap_or_default() {
            show_row(ui, palette, row, hide);
        }
    });
}

fn show_row(ui: &mut Ui, palette: &Palette, row: &Row, hide: bool) {
    let (sign, color) = match row.mark {
        Mark::Same => (" ", palette.text_muted),
        Mark::Removed => ("-", palette.error),
        Mark::Added => ("+", palette.ok),
        Mark::Skipped => ("…", palette.text_faint),
    };
    let number = |line: Option<usize>| line.map_or_else(|| " ".repeat(5), |line| format!("{line:>5}"));
    let shown = row.hidden.as_deref().filter(|_| hide).unwrap_or(&row.text);
    let text = format!("{} {} {sign} {}", number(row.old), number(row.new), short(shown, ROW_LIMIT));
    ui.label(RichText::new(text).monospace().color(color));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// "-3 b": the mark, the numbers and the text of each row.
    fn shown(old: &str, new: &str) -> Vec<String> {
        let sign = |mark| match mark {
            Mark::Same => " ",
            Mark::Removed => "-",
            Mark::Added => "+",
            Mark::Skipped => "~",
        };
        let number = |line: Option<usize>| line.map_or("_".to_owned(), |line| line.to_string());
        rows(old, new, false)
            .into_iter()
            .map(|row| format!("{}{} {} {}", sign(row.mark), number(row.old), number(row.new), row.text))
            .collect()
    }

    #[test]
    fn each_change_shows_with_three_lines_of_context_and_the_other_lines_as_a_count() {
        let old: String = (1..=20).map(|line| format!("l{line}\n")).collect();
        let new = old.replace("l5\n", "L5\n").replace("l18\n", "l18\nnew\n");
        assert_eq!(
            shown(&old, &new),
            [
                "~_ _ 1 line with no change",
                " 2 2 l2",
                " 3 3 l3",
                " 4 4 l4",
                "-5 _ l5",
                "+_ 5 L5",
                " 6 6 l6",
                " 7 7 l7",
                " 8 8 l8",
                "~_ _ 7 lines with no change",
                " 16 16 l16",
                " 17 17 l17",
                " 18 18 l18",
                "+_ 19 new",
                " 19 20 l19",
                " 20 21 l20",
            ]
        );
        assert_eq!(shown(&old, &old), Vec::<String>::new());
    }

    #[test]
    fn changes_closer_than_the_context_share_their_lines() {
        let old = "a\nb\nc\nd\ne\nf\ng\nh\n";
        assert_eq!(
            shown(old, "a\nB\nc\nd\ne\nF\ng\nh\n"),
            [" 1 1 a", "-2 _ b", "+_ 2 B", " 3 3 c", " 4 4 d", " 5 5 e", "-6 _ f", "+_ 6 F", " 7 7 g", " 8 8 h"]
        );
        assert_eq!(shown("a\n", ""), ["-1 _ a"]);
        assert_eq!(shown("a", "a\n"), ["-1 _ a  (no line feed at the end)", "+_ 1 a"]);
    }
}
