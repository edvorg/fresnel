//! The rows of the pane: the lines that it shows, and the rows of one line.
//!
//! A position is a line number and a row of that line, so the pane stays on its line when
//! older lines leave the ring and new lines come. A row has a fixed count of columns: with
//! a monospace font, the rows of a line follow from its count of characters, with no layout.
//! A line counts its characters up to the limit of its laid-out part only, so the cost of a
//! frame does not grow with the length of a line.

use std::collections::VecDeque;
use std::ops::Range;

use fresnel_core::logs::{Lines, LogLine};

/// The most characters of one line that the pane lays out. A longer line shows its start
/// and a row with a button for the full line: the layout of a line of 64 KiB takes
/// milliseconds, and a screen of them would hold the frame.
pub(crate) const LAID_OUT: usize = 2_000;

/// The lines that the pane shows: each line of the view, or the lines that a filter kept.
#[derive(Clone, Copy)]
pub(crate) struct Shown<'a> {
    pub(crate) lines: &'a Lines,
    /// The numbers of the kept lines, in their sequence. `None` with no filter.
    pub(crate) kept: Option<&'a VecDeque<u64>>,
}

impl<'a> Shown<'a> {
    pub(crate) fn len(self) -> usize {
        self.kept.map_or(self.lines.len(), VecDeque::len)
    }

    /// The number of the shown line at this index.
    pub(crate) fn number(self, index: usize) -> Option<u64> {
        match self.kept {
            Some(kept) => kept.get(index).copied(),
            None => (index < self.lines.len()).then(|| self.lines.first() + index as u64),
        }
    }

    /// The index of the first shown line with this number or a later one: `len` when no
    /// such line exists.
    pub(crate) fn index_from(self, number: u64) -> usize {
        match self.kept {
            Some(kept) => kept.partition_point(|kept| *kept < number),
            None => usize::try_from(number.saturating_sub(self.lines.first())).unwrap_or(usize::MAX).min(self.len()),
        }
    }

    /// The shown line at this index, with its number.
    pub(crate) fn line(self, index: usize) -> Option<(u64, &'a LogLine)> {
        let number = self.number(index)?;
        Some((number, self.lines.get(number)?))
    }
}

/// One row of a line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Row {
    /// The bytes of the text of the line in the row.
    pub(crate) bytes: Range<usize>,
    /// True for the row after the laid-out part of a long line, with its button.
    pub(crate) more: bool,
}

/// How the pane cuts its lines into rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Cut {
    /// The columns of a row with wrap; `None` with no wrap: one row for each line.
    pub(crate) columns: Option<usize>,
}

impl Cut {
    /// The rows of a line: one for each `columns` characters of its laid-out part, at least
    /// one, and one more for a long line or a line that the core cut.
    pub(crate) fn rows(self, line: &LogLine) -> Vec<Row> {
        let text = &*line.text;
        let (end, long) = laid_out(text);
        let mut rows = Vec::new();
        match self.columns {
            Some(columns) => {
                let mut start = 0;
                let mut count = 0;
                for (byte, _) in text.char_indices().take(LAID_OUT) {
                    if count == columns.max(1) {
                        rows.push(Row { bytes: start..byte, more: false });
                        (start, count) = (byte, 0);
                    }
                    count += 1;
                }
                rows.push(Row { bytes: start..end, more: false });
            }
            None => rows.push(Row { bytes: 0..end, more: false }),
        }
        if long || line.cut {
            rows.push(Row { bytes: end..end, more: true });
        }
        rows
    }

    /// The count of rows of a line, as `rows` gives them.
    pub(crate) fn count(self, line: &LogLine) -> usize {
        let text = &*line.text;
        let chars = text.chars().take(LAID_OUT + 1).count();
        let more = usize::from(chars > LAID_OUT || line.cut);
        match self.columns {
            Some(columns) => chars.min(LAID_OUT).div_ceil(columns.max(1)).max(1) + more,
            None => 1 + more,
        }
    }
}

/// The end of the laid-out part of a text, and true when the text is longer.
pub(crate) fn laid_out(text: &str) -> (usize, bool) {
    match text.char_indices().nth(LAID_OUT) {
        Some((byte, _)) => (byte, true),
        None => (text.len(), false),
    }
}

/// The top of the pane, or some other row: a row of a shown line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Position {
    pub(crate) line: u64,
    pub(crate) row: usize,
}

/// The position `rows` rows after `from` (before it for a negative count), over the shown
/// lines. It stops at the first row of the first line and at the last row of the last one.
pub(crate) fn walk(shown: Shown<'_>, cut: Cut, from: Position, rows: isize) -> Position {
    let mut index = shown.index_from(from.line);
    let Some((mut number, mut line)) = shown.line(index).or_else(|| last(shown)) else {
        return from;
    };
    index = index.min(shown.len().saturating_sub(1));
    let mut row = if number == from.line { from.row.min(cut.count(line) - 1) } else { 0 };
    let mut left = rows.unsigned_abs();
    if rows >= 0 {
        loop {
            let room = cut.count(line) - 1 - row;
            if left <= room {
                return Position { line: number, row: row + left };
            }
            left -= room + 1;
            let Some(next) = shown.line(index + 1) else {
                return Position { line: number, row: cut.count(line) - 1 };
            };
            (index, (number, line), row) = (index + 1, next, 0);
        }
    }
    loop {
        if left <= row {
            return Position { line: number, row: row - left };
        }
        left -= row + 1;
        let Some(before) = index.checked_sub(1).and_then(|before| shown.line(before)) else {
            return Position { line: number, row: 0 };
        };
        (index, (number, line)) = (index - 1, before);
        row = cut.count(line) - 1;
    }
}

/// The last row of the last shown line.
pub(crate) fn end(shown: Shown<'_>, cut: Cut) -> Option<Position> {
    let (number, line) = last(shown)?;
    Some(Position { line: number, row: cut.count(line) - 1 })
}

fn last<'a>(shown: Shown<'a>) -> Option<(u64, &'a LogLine)> {
    shown.line(shown.len().checked_sub(1)?)
}

#[cfg(test)]
mod tests;
