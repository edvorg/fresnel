//! The text of an editor as a list of lines, and its one change: a range that becomes an
//! other text. A key press changes one line, so the text needs no copy of the rest, and a
//! frame finds the lines on the screen by their numbers.

use std::ops::Range;

/// A place in the text: a line, and a byte of it at the start of a character or at its end.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct Pos {
    pub(crate) line: usize,
    pub(crate) byte: usize,
}

impl Pos {
    pub(crate) const fn new(line: usize, byte: usize) -> Self {
        Self { line, byte }
    }
}

/// One change: `removed` at `at` became `inserted`. The undo applies its reverse.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Change {
    pub(crate) at: Pos,
    pub(crate) removed: String,
    pub(crate) inserted: String,
}

impl Change {
    /// The change that takes this one back.
    pub(crate) fn reverse(&self) -> Self {
        Self { at: self.at, removed: self.inserted.clone(), inserted: self.removed.clone() }
    }
}

/// The lines of a text, with a number that grows with each change.
#[derive(Debug)]
pub(crate) struct Buffer {
    /// Never empty: an empty text has one empty line. A text that ends with a line feed has
    /// an empty last line.
    lines: Vec<String>,
    generation: u64,
}

impl Buffer {
    pub(crate) fn new(text: &str) -> Self {
        Self { lines: text.split('\n').map(str::to_owned).collect(), generation: 0 }
    }

    /// The line with this number, or an empty one past the end.
    pub(crate) fn line(&self, line: usize) -> &str {
        self.lines.get(line).map_or("", String::as_str)
    }

    /// The count of lines, with an empty last line.
    pub(crate) fn count(&self) -> usize {
        self.lines.len()
    }

    /// The count of lines as a user counts them: an empty last line is the end of the line before.
    pub(crate) fn text_lines(&self) -> usize {
        let empty_end = self.lines.len() > 1 && self.lines.last().is_some_and(String::is_empty);
        self.lines.len() - usize::from(empty_end)
    }

    /// A number that grows with each change of the text.
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn text(&self) -> String {
        self.lines.join("\n")
    }

    /// The place after the last character.
    pub(crate) fn end(&self) -> Pos {
        let line = self.lines.len().saturating_sub(1);
        Pos::new(line, self.line(line).len())
    }

    /// The nearest place in the text at the start of a character.
    pub(crate) fn clamp(&self, pos: Pos) -> Pos {
        if pos.line >= self.lines.len() {
            return self.end();
        }
        let line = self.line(pos.line);
        let mut byte = pos.byte.min(line.len());
        while !line.is_char_boundary(byte) {
            byte -= 1;
        }
        Pos::new(pos.line, byte)
    }

    /// The text of a range.
    pub(crate) fn slice(&self, range: Range<Pos>) -> String {
        let (start, end) = (self.clamp(range.start), self.clamp(range.end));
        if start >= end {
            return String::new();
        }
        if start.line == end.line {
            return self.line(start.line).get(start.byte..end.byte).unwrap_or_default().to_owned();
        }
        let mut text = self.line(start.line).get(start.byte..).unwrap_or_default().to_owned();
        for line in start.line + 1..end.line {
            text.push('\n');
            text.push_str(self.line(line));
        }
        text.push('\n');
        text.push_str(self.line(end.line).get(..end.byte).unwrap_or_default());
        text
    }

    /// Replaces a range with a text. Gives the change, for the undo.
    pub(crate) fn replace(&mut self, range: Range<Pos>, text: &str) -> Change {
        let (start, end) = (self.clamp(range.start), self.clamp(range.end));
        let (start, end) = if start <= end { (start, end) } else { (end, start) };
        let change = Change { at: start, removed: self.slice(start..end), inserted: text.to_owned() };
        self.apply(&change);
        change
    }

    /// Applies a change whose `removed` is the text at its place. Gives the place after its
    /// inserted text.
    pub(crate) fn apply(&mut self, change: &Change) -> Pos {
        let start = self.clamp(change.at);
        let end = self.clamp(end_of(start, &change.removed));
        let head = self.line(start.line).get(..start.byte).unwrap_or_default();
        let tail = self.line(end.line).get(end.byte..).unwrap_or_default();
        let mut new: Vec<String> = change.inserted.split('\n').map(str::to_owned).collect();
        if let Some(first) = new.first_mut() {
            first.insert_str(0, head);
        }
        if let Some(last) = new.last_mut() {
            last.push_str(tail);
        }
        let last = self.lines.len().saturating_sub(1);
        self.lines.splice(start.line.min(last)..=end.line.min(last), new);
        self.generation += 1;
        end_of(start, &change.inserted)
    }
}

/// The place after a text that starts at `at`.
pub(crate) fn end_of(at: Pos, text: &str) -> Pos {
    match text.rfind('\n') {
        None => Pos::new(at.line, at.byte + text.len()),
        Some(last) => Pos::new(at.line + text.matches('\n').count(), text.len() - last - 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_replace_of_a_range_changes_the_lines_and_its_reverse_gives_the_text_back() {
        let mut buffer = Buffer::new("kind: Pod\nmetadata:\n  name: web\n");
        assert_eq!((buffer.count(), buffer.text_lines()), (4, 3));
        let change = buffer.replace(Pos::new(1, 9)..Pos::new(2, 7), "\n  labels:\n    app: web\n  name:");
        assert_eq!(buffer.text(), "kind: Pod\nmetadata:\n  labels:\n    app: web\n  name: web\n");
        assert_eq!(change.removed, "\n  name:");
        assert_eq!(buffer.generation(), 1);
        let back = buffer.apply(&change.reverse());
        assert_eq!((buffer.text().as_str(), back), ("kind: Pod\nmetadata:\n  name: web\n", Pos::new(2, 7)));
        assert_eq!(buffer.slice(Pos::new(0, 6)..Pos::new(1, 4)), "Pod\nmeta");
    }

    #[test]
    fn a_place_stays_at_the_start_of_a_character_and_inside_the_text() {
        let buffer = Buffer::new("größe\nx");
        assert_eq!(buffer.clamp(Pos::new(0, 3)), Pos::new(0, 2), "inside the ö");
        assert_eq!(buffer.clamp(Pos::new(0, 99)), Pos::new(0, 7));
        assert_eq!(buffer.clamp(Pos::new(5, 0)), Pos::new(1, 1), "past the last line");
        assert_eq!(Buffer::new("").text_lines(), 1);
        assert_eq!(end_of(Pos::new(3, 4), "ab\ncde\nf"), Pos::new(5, 1));
    }
}
