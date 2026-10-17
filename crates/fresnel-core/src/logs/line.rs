//! One line of a log view, with the styles of its text.

use std::sync::Arc;

/// What a line of a view is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    /// A line of a container log.
    Log,
    /// A line of the app between the lines of a source: its stream or its container started
    /// again, or its stream ended. The text tells what occurred.
    Marker,
}

/// One line of a log view. A clone copies pointers only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogLine {
    /// The time in nanoseconds since 1970 (UTC), which the container runtime wrote before the
    /// line. A marker has the time of the last line of its source before it, so it stays at
    /// its place in the sequence of time.
    pub time: i64,
    /// The index of the source of the line in `LogView::sources`.
    pub source: u32,
    /// The text with no timestamp, no line end and no escape sequence. A search reads this
    /// text, so a color code in a word does not hide the word.
    pub text: Arc<str>,
    /// The styles of the text, in the sequence of their start. `None` for plain text.
    pub spans: Option<Arc<[Span]>>,
    /// A log line or a marker.
    pub kind: LineKind,
    /// True for a line that arrived after a later line of an other source went into the
    /// view: the line is at the end of the view, not at its place in time.
    pub late: bool,
    /// True for a line that was longer than the limit of one line: the view has its start.
    pub cut: bool,
}

impl LogLine {
    /// A marker of the app for a source, at the time `time`.
    pub(crate) fn marker(time: i64, source: u32, text: impl Into<Arc<str>>) -> Self {
        Self { time, source, text: text.into(), spans: None, kind: LineKind::Marker, late: false, cut: false }
    }

    /// The bytes that the line holds in memory: the limit of a view counts them. The text and
    /// the styles are each an allocation with the two counts of its shared pointer, and the
    /// line has its place in a chunk of the ring. With the line alone, a ring of short lines
    /// held a third more than its limit in a soak.
    pub(crate) fn weight(&self) -> usize {
        let spans = self.spans.as_ref().map_or(0, |spans| SHARED + spans.len() * size_of::<Span>());
        PLACE + SHARED + self.text.len() + spans
    }
}

/// The two counts of a shared pointer before its value.
const SHARED: usize = 2 * size_of::<usize>();
/// The place of a line in the vector of `imbl`: the chunks and the nodes of the tree add a
/// sixth to the line, as the counting allocator measured 56 bytes for a line of 48.
const PLACE: usize = size_of::<LogLine>() + size_of::<LogLine>() / 6;

/// One style in a line: it holds from `start` to the start of the next span or to the end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    /// The byte offset in `LogLine::text`. It is always at the start of a character.
    pub start: u32,
    /// The style.
    pub style: Style,
}

/// The style of a part of a line, from the SGR codes of the program. The default is plain
/// text in the colors of the view.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style {
    /// The color of the text. `None` for the color of the view.
    pub foreground: Option<Color>,
    /// The color behind the text. `None` for the color of the view.
    pub background: Option<Color>,
    /// Bold text.
    pub bold: bool,
    /// Text with less contrast.
    pub dim: bool,
    /// Italic text.
    pub italic: bool,
    /// Text with a line below it.
    pub underline: bool,
    /// The two colors change places.
    pub inverse: bool,
    /// Text with a line through it.
    pub strike: bool,
}

/// A color of the program.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Color {
    /// An entry of the palette of 256 colors: 0 to 7 are the standard colors, 8 to 15 their
    /// bright forms, 16 to 231 a cube of colors and 232 to 255 grays. The view selects them.
    Palette(u8),
    /// A color with its red, green and blue values.
    Rgb(u8, u8, u8),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ring of short lines held a third more than its limit in a soak when the weight
    /// counted the line alone: 81 bytes for a line that weighed 58.
    #[test]
    fn the_weight_of_a_line_counts_its_text_its_styles_and_the_counts_of_their_pointers() {
        let mut line = LogLine::marker(0, 0, "soak 123");
        let place = size_of::<LogLine>() * 7 / 6;
        assert_eq!(line.weight(), place + 2 * size_of::<usize>() + 8);
        line.spans = Some(vec![Span { start: 0, style: Style::default() }; 2].into());
        let spans = 2 * size_of::<usize>() + 2 * size_of::<Span>();
        assert_eq!(line.weight(), place + 2 * size_of::<usize>() + 8 + spans);
    }
}
