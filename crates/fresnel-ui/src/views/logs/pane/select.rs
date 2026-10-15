//! The selection of the pane and its text (research 05 section 5.6).
//!
//! The label selection of egui only knows the rows on the screen. The pane has its own: an
//! anchor and a cursor, each a line number and a byte in its text, so a selection can span
//! lines that left the screen, and it stays on its lines when older lines leave the ring.

use std::ops::Range;
use std::sync::Arc;

use fresnel_core::logs::{LineKind, LogLine, SourceView};
use k8s_openapi::jiff::Timestamp;

use super::rows::Shown;

/// A place in the text of the view: a line and a byte at the start of a character.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Point {
    pub(crate) line: u64,
    pub(crate) byte: usize,
}

/// The selection: from the anchor, where the drag started, to the cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Selection {
    pub(crate) anchor: Point,
    pub(crate) cursor: Point,
}

impl Selection {
    /// The start and the end, in the sequence of the text.
    pub(crate) fn ordered(self) -> (Point, Point) {
        if self.anchor <= self.cursor { (self.anchor, self.cursor) } else { (self.cursor, self.anchor) }
    }

    pub(crate) fn is_empty(self) -> bool {
        self.anchor == self.cursor
    }

    /// The selected bytes of a line of `len` bytes. `None` for a line outside the selection.
    pub(crate) fn of_line(self, number: u64, len: usize) -> Option<Range<usize>> {
        let (start, end) = self.ordered();
        if self.is_empty() || number < start.line || number > end.line {
            return None;
        }
        let from = if number == start.line { start.byte.min(len) } else { 0 };
        let to = if number == end.line { end.byte.min(len) } else { len };
        Some(from..to)
    }
}

/// The columns that a copy writes before the text of a line, as the pane shows them.
#[derive(Clone, Debug, Default)]
pub(crate) struct Columns {
    /// The time of each line, in the form of `kubectl logs --timestamps`.
    pub(crate) times: bool,
    /// The sources of a view with more than one, for the form of `kubectl logs --prefix`.
    pub(crate) sources: Option<Arc<[SourceView]>>,
}

/// The text of a selection: the selected part of each shown line in it, one line for each.
/// A line whose start is in the selection gets the columns before it. The text stops after
/// `limit` bytes. Gives the text, its count of lines and true when it stopped.
pub(crate) fn text(shown: Shown<'_>, selection: Selection, columns: &Columns, limit: usize) -> (String, u64, bool) {
    let (start, end) = selection.ordered();
    let mut out = String::new();
    let mut count = 0;
    let mut index = shown.index_from(start.line);
    while let Some((number, line)) = shown.line(index).filter(|(number, _)| *number <= end.line) {
        let Some(part) = selection
            .of_line(number, line.text.len())
            .and_then(|bytes| line.text.get(bytes.clone()).map(|text| (bytes.start, text)))
        else {
            index += 1;
            continue;
        };
        if count > 0 {
            out.push('\n');
        }
        if part.0 == 0 {
            head(&mut out, line, columns);
        }
        out.push_str(part.1);
        count += 1;
        if out.len() >= limit {
            out.truncate(floor(&out, limit));
            return (out, count, true);
        }
        index += 1;
    }
    (out, count, false)
}

/// The columns of one line: "[pod/web-1/app] 2026-10-12T20:00:00.987654321Z ".
fn head(out: &mut String, line: &LogLine, columns: &Columns) {
    let source = columns.sources.as_ref().and_then(|sources| sources.get(usize::try_from(line.source).ok()?));
    if let Some(source) = source {
        out.push_str(&format!("[pod/{}/{}] ", source.pod.name, source.container));
    }
    if columns.times {
        match Timestamp::from_nanosecond(i128::from(line.time)) {
            Ok(time) => out.push_str(&format!("{time:.9} ")),
            Err(_) => out.push_str("- "),
        }
    }
    if line.kind == LineKind::Marker {
        out.push_str("--- ");
    }
}

/// The largest start of a character at or before `limit`.
fn floor(text: &str, limit: usize) -> usize {
    (0..=limit.min(text.len())).rev().find(|byte| text.is_char_boundary(*byte)).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use fresnel_core::logs::{Lines, SourceState};
    use fresnel_core::store::ObjectKey;

    use super::*;

    fn line(seconds: i64, source: u32, text: &str) -> LogLine {
        let time = (1_760_299_200 + seconds) * 1_000_000_000;
        LogLine { time, source, text: text.into(), spans: None, kind: LineKind::Log, late: false, cut: false }
    }

    fn at(line: u64, byte: usize) -> Point {
        Point { line, byte }
    }

    #[test]
    fn a_selection_has_the_part_of_each_line_from_its_start_to_its_end_in_either_direction() {
        let selection = Selection { anchor: at(7, 2), cursor: at(5, 3) };
        assert_eq!(selection.ordered(), (at(5, 3), at(7, 2)));
        assert_eq!(
            [4, 5, 6, 7, 8].map(|number| selection.of_line(number, 10)),
            [None, Some(3..10), Some(0..10), Some(0..2), None]
        );
        assert_eq!(Selection { anchor: at(5, 3), cursor: at(5, 3) }.of_line(5, 10), None, "an empty one");
        assert_eq!(Selection { anchor: at(5, 30), cursor: at(6, 1) }.of_line(5, 10), Some(10..10));
    }

    #[test]
    fn a_copy_has_the_shown_lines_with_the_columns_of_the_pane_and_stops_at_its_limit() {
        let lines = Lines::new(0, [line(0, 0, "alpha one"), line(1, 1, "beta two"), line(2, 0, "gamma three")]);
        let selection = Selection { anchor: at(0, 6), cursor: at(2, 5) };
        let plain = Columns::default();
        let all = Shown { lines: &lines, kept: None };
        assert_eq!(text(all, selection, &plain, 1000), ("one\nbeta two\ngamma".to_owned(), 3, false));
        let source = |pod: &str| SourceView {
            pod: ObjectKey::new("shop", pod),
            container: "app".into(),
            state: SourceState::Streaming,
            lines: 0,
            repeated: 0,
        };
        let columns = Columns { times: true, sources: Some(Arc::from([source("web-1"), source("web-2")])) };
        let kept = VecDeque::from([0, 2]);
        let filtered = Shown { lines: &lines, kept: Some(&kept) };
        assert_eq!(
            text(filtered, selection, &columns, 1000).0,
            "one\n[pod/web-1/app] 2025-10-12T20:00:02.000000000Z gamma",
            "the line that the filter hides is not in the copy"
        );
        assert_eq!(text(all, selection, &plain, 6), ("one\nbe".to_owned(), 2, true));
        let wide = Lines::new(0, [line(0, 0, "größer")]);
        let whole = Selection { anchor: at(0, 0), cursor: at(0, 8) };
        assert_eq!(text(Shown { lines: &wide, kept: None }, whole, &plain, 3).0, "gr", "a cut between two characters");
    }
}
