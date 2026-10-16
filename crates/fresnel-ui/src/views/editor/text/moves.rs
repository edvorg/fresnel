//! The selection of an editor and the moves of its cursor in the text: by a character, a
//! word, a line, a page, and to the start or the end.

use std::ops::Range;

use super::buffer::{Buffer, Pos};

/// A selection: the place where it started and the place of the cursor. An empty selection
/// is a cursor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Selection {
    pub(crate) anchor: Pos,
    pub(crate) head: Pos,
}

impl Selection {
    pub(crate) const fn at(pos: Pos) -> Self {
        Self { anchor: pos, head: pos }
    }

    /// The selected range, its start first.
    pub(crate) fn range(&self) -> Range<Pos> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.anchor == self.head
    }
}

/// A move of the cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Motion {
    Left,
    Right,
    WordLeft,
    WordRight,
    Up,
    Down,
    PageUp,
    PageDown,
    LineStart,
    LineEnd,
    Start,
    End,
}

/// The place of a move. `goal` is the column that up and down keep, `page` the count of
/// lines of a page.
pub(crate) fn go(buffer: &Buffer, from: Pos, motion: Motion, goal: usize, page: usize) -> Pos {
    let line = buffer.line(from.line);
    match motion {
        Motion::Left => match previous_boundary(line, from.byte) {
            Some(byte) => Pos::new(from.line, byte),
            None if from.line > 0 => Pos::new(from.line - 1, buffer.line(from.line - 1).len()),
            None => from,
        },
        Motion::Right => match next_boundary(line, from.byte) {
            Some(byte) => Pos::new(from.line, byte),
            None if from.line + 1 < buffer.count() => Pos::new(from.line + 1, 0),
            None => from,
        },
        Motion::WordLeft => word_left(buffer, from),
        Motion::WordRight => word_right(buffer, from),
        Motion::Up => vertical(buffer, from.line.saturating_sub(1), goal, from.line == 0, Pos::new(0, 0)),
        Motion::Down => vertical(buffer, from.line + 1, goal, from.line + 1 >= buffer.count(), buffer.end()),
        Motion::PageUp => vertical(buffer, from.line.saturating_sub(page), goal, from.line == 0, Pos::new(0, 0)),
        Motion::PageDown => {
            let last = buffer.count().saturating_sub(1);
            vertical(buffer, (from.line + page).min(last), goal, from.line == last, buffer.end())
        }
        Motion::LineStart => {
            // The first press goes to the text after the indent, the next one to column 0.
            let text = line.len() - line.trim_start().len();
            Pos::new(from.line, if from.byte == text { 0 } else { text })
        }
        Motion::LineEnd => Pos::new(from.line, line.len()),
        Motion::Start => Pos::new(0, 0),
        Motion::End => buffer.end(),
    }
}

/// The place in a line at a column, or `edge` when the move leaves the text.
fn vertical(buffer: &Buffer, line: usize, goal: usize, past: bool, edge: Pos) -> Pos {
    if past {
        return edge;
    }
    Pos::new(line, byte_of_column(buffer.line(line), goal))
}

/// The column of a place in a line: its count of characters before it.
pub(crate) fn column(line: &str, byte: usize) -> usize {
    line.get(..byte).map_or(0, |part| part.chars().count())
}

/// The byte of a column in a line, or the end of a shorter line.
pub(crate) fn byte_of_column(line: &str, column: usize) -> usize {
    line.char_indices().nth(column).map_or(line.len(), |(byte, _)| byte)
}

fn previous_boundary(line: &str, byte: usize) -> Option<usize> {
    line.get(..byte)?.char_indices().next_back().map(|(index, _)| index)
}

fn next_boundary(line: &str, byte: usize) -> Option<usize> {
    let character = line.get(byte..)?.chars().next()?;
    Some(byte + character.len_utf8())
}

/// A character of a word: a letter, a digit or `_`, as in other editors.
fn is_word(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

/// The start of the word before a place, over the end of the line before.
fn word_left(buffer: &Buffer, from: Pos) -> Pos {
    if from.byte == 0 {
        return go(buffer, from, Motion::Left, 0, 0);
    }
    let line = buffer.line(from.line);
    let before = line.get(..from.byte).unwrap_or_default();
    let trimmed = before.trim_end_matches(|character: char| !is_word(character));
    let start = trimmed.trim_end_matches(is_word).len();
    Pos::new(from.line, start)
}

/// The end of the word after a place, over the start of the next line.
fn word_right(buffer: &Buffer, from: Pos) -> Pos {
    let line = buffer.line(from.line);
    if from.byte >= line.len() {
        return go(buffer, from, Motion::Right, 0, 0);
    }
    let after = line.get(from.byte..).unwrap_or_default();
    let gap = after.len() - after.trim_start_matches(|character: char| !is_word(character)).len();
    let word = after.get(gap..).unwrap_or_default();
    let end = word.len() - word.trim_start_matches(is_word).len();
    Pos::new(from.line, from.byte + gap + end)
}

/// The word at a place, for a double click: its letters, or the one character there.
pub(crate) fn word_at(buffer: &Buffer, at: Pos) -> Range<Pos> {
    let line = buffer.line(at.line);
    let (before, after) = (line.get(..at.byte).unwrap_or_default(), line.get(at.byte..).unwrap_or_default());
    let start = before.trim_end_matches(is_word).len();
    let end = at.byte + after.len() - after.trim_start_matches(is_word).len();
    if start == end {
        return at..Pos::new(at.line, next_boundary(line, at.byte).unwrap_or(at.byte));
    }
    Pos::new(at.line, start)..Pos::new(at.line, end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cursor_moves_by_character_word_line_and_page_and_keeps_its_column() {
        let buffer = Buffer::new("  name: web-1\nx\n  image: nginx\n");
        let at = |motion, from: Pos, goal| go(&buffer, from, motion, goal, 2);
        assert_eq!(at(Motion::Left, Pos::new(1, 0), 0), Pos::new(0, 13), "over the end of the line before");
        assert_eq!(at(Motion::Right, Pos::new(0, 13), 0), Pos::new(1, 0));
        assert_eq!(at(Motion::WordRight, Pos::new(0, 0), 0), Pos::new(0, 6), "the end of `name`");
        assert_eq!(at(Motion::WordRight, Pos::new(0, 6), 0), Pos::new(0, 11), "the end of `web`");
        assert_eq!(at(Motion::WordLeft, Pos::new(0, 13), 0), Pos::new(0, 12), "the start of `1`");
        assert_eq!(at(Motion::WordLeft, Pos::new(0, 12), 0), Pos::new(0, 8));
        assert_eq!(at(Motion::Down, Pos::new(0, 9), 9), Pos::new(1, 1), "the end of a short line");
        assert_eq!(at(Motion::Down, Pos::new(1, 1), 9), Pos::new(2, 9), "the column comes back");
        assert_eq!(at(Motion::Up, Pos::new(0, 4), 4), Pos::new(0, 0), "up from the first line");
        assert_eq!(at(Motion::PageDown, Pos::new(0, 0), 0), Pos::new(2, 0));
        assert_eq!(at(Motion::LineStart, Pos::new(2, 9), 0), Pos::new(2, 2), "the text after the indent");
        assert_eq!(at(Motion::LineStart, Pos::new(2, 2), 0), Pos::new(2, 0), "then column 0");
        assert_eq!(at(Motion::End, Pos::new(0, 0), 0), Pos::new(3, 0));
    }

    #[test]
    fn a_double_click_selects_a_word_or_one_character() {
        let buffer = Buffer::new("app.kubernetes.io/name: größe");
        assert_eq!(word_at(&buffer, Pos::new(0, 6)), Pos::new(0, 4)..Pos::new(0, 14));
        assert_eq!(word_at(&buffer, Pos::new(0, 3)), Pos::new(0, 0)..Pos::new(0, 3), "the word that ends there");
        assert_eq!(word_at(&buffer, Pos::new(0, 23)), Pos::new(0, 23)..Pos::new(0, 24), "a space between no words");
        assert_eq!(word_at(&buffer, Pos::new(0, 26)), Pos::new(0, 24)..Pos::new(0, 31));
        assert_eq!((column("größe", 4), byte_of_column("größe", 3)), (3, 4));
    }
}
