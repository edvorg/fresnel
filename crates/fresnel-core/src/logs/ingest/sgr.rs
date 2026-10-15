//! The escape sequences of a line: an SGR code gives a style, and each other sequence and
//! control byte goes (research 05, section 5.3).
//!
//! The parser of `vte` reads the bytes, also the UTF-8, and gives each character and each
//! SGR code to the [`Pen`]. The style holds from one line of a source to the next, as in a
//! terminal: a program can set a color, write three lines and then reset it.

use std::sync::Arc;

use vte::ansi::{self, Attr, Handler, NamedColor, Processor};

use crate::logs::line::{Color, Span, Style};

/// The style of one source between two lines.
#[derive(Default)]
pub(super) struct Pen {
    style: Style,
}

impl Pen {
    /// The text of a line and its styles. A line with no control byte, written in the
    /// default style, skips the parser.
    pub(super) fn draw(&mut self, bytes: &[u8]) -> (Arc<str>, Option<Arc<[Span]>>) {
        if self.style == Style::default() && bytes.iter().all(|byte| (*byte >= b' ' && *byte != 0x7f) || *byte == b'\t')
        {
            return (String::from_utf8_lossy(bytes).into(), None);
        }
        let mut writer = Writer { text: String::with_capacity(bytes.len()), spans: Vec::new(), style: &mut self.style };
        let mut parser: Processor = Processor::new();
        parser.advance(&mut writer, bytes);
        // A program can start a synchronized update (`CSI ? 2026 h`). The parser then keeps
        // the bytes for a later end, and the line would lose them.
        if ansi::Timeout::pending_timeout(parser.sync_timeout()) {
            parser.stop_sync(&mut writer);
        }
        let Writer { text, spans, .. } = writer;
        let styled = spans.iter().any(|span| span.style != Style::default());
        (text.into(), styled.then(|| spans.into()))
    }
}

/// The text and the styles of one line.
struct Writer<'a> {
    text: String,
    spans: Vec<Span>,
    style: &'a mut Style,
}

impl Handler for Writer<'_> {
    fn input(&mut self, character: char) {
        let current = self.spans.last().map_or(Style::default(), |span| span.style);
        if current != *self.style {
            // The text of a line is shorter than the limit of one line, far below 4 GiB.
            let start = u32::try_from(self.text.len()).unwrap_or(u32::MAX);
            self.spans.push(Span { start, style: *self.style });
        }
        self.text.push(character);
    }

    fn put_tab(&mut self, count: u16) {
        for _ in 0..count {
            self.input('\t');
        }
    }

    fn terminal_attribute(&mut self, attribute: Attr) {
        let style = &mut *self.style;
        match attribute {
            Attr::Reset => *style = Style::default(),
            Attr::Bold => style.bold = true,
            Attr::Dim => style.dim = true,
            Attr::Italic => style.italic = true,
            Attr::Underline
            | Attr::DoubleUnderline
            | Attr::Undercurl
            | Attr::DottedUnderline
            | Attr::DashedUnderline => style.underline = true,
            Attr::Reverse => style.inverse = true,
            Attr::Strike => style.strike = true,
            Attr::CancelBold => style.bold = false,
            Attr::CancelBoldDim => (style.bold, style.dim) = (false, false),
            Attr::CancelItalic => style.italic = false,
            Attr::CancelUnderline => style.underline = false,
            Attr::CancelReverse => style.inverse = false,
            Attr::CancelStrike => style.strike = false,
            Attr::Foreground(color) => style.foreground = color_of(color),
            Attr::Background(color) => style.background = color_of(color),
            // A blink, a hidden text and the color of an underline have no form in the view.
            Attr::BlinkSlow
            | Attr::BlinkFast
            | Attr::Hidden
            | Attr::CancelBlink
            | Attr::CancelHidden
            | Attr::UnderlineColor(_) => {}
        }
    }
}

/// The color of the view for a color of the parser. `None` is the default color.
fn color_of(color: ansi::Color) -> Option<Color> {
    match color {
        ansi::Color::Spec(rgb) => Some(Color::Rgb(rgb.r, rgb.g, rgb.b)),
        ansi::Color::Indexed(index) => Some(Color::Palette(index)),
        ansi::Color::Named(named) => {
            let number = named as usize;
            let dim = NamedColor::DimBlack as usize..=NamedColor::DimWhite as usize;
            let entry = if dim.contains(&number) { number - dim.start() } else { number };
            u8::try_from(entry).ok().filter(|entry| *entry < 16).map(Color::Palette)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(start: u32, style: Style) -> Span {
        Span { start, style }
    }

    fn red() -> Style {
        Style { foreground: Some(Color::Palette(1)), ..Style::default() }
    }

    #[test]
    fn plain_text_has_no_span_and_keeps_each_character() {
        let mut pen = Pen::default();
        let (text, spans) = pen.draw("plain\ttext äö 漢字".as_bytes());
        assert_eq!((&*text, spans), ("plain\ttext äö 漢字", None));
        let (text, _) = pen.draw(b"bad \xff byte");
        assert_eq!(&*text, "bad \u{fffd} byte");
    }

    #[test]
    fn sgr_codes_give_spans_and_the_text_has_no_escape_sequence() {
        let mut pen = Pen::default();
        let (text, spans) = pen.draw(b"a \x1b[31mred\x1b[0m and \x1b[1;38;5;208mbold orange\x1b[m.");
        assert_eq!(&*text, "a red and bold orange.");
        let orange = Style { foreground: Some(Color::Palette(208)), bold: true, ..Style::default() };
        let expected = [span(2, red()), span(5, Style::default()), span(10, orange), span(21, Style::default())];
        assert_eq!(spans.as_deref(), Some(&expected[..]));

        let (text, spans) = pen.draw(b"\x1b[48;2;1;2;3mx\x1b[92;4my\x1b[2;37mz");
        assert_eq!(&*text, "xyz");
        let background = Some(Color::Rgb(1, 2, 3));
        let first = Style { background, ..Style::default() };
        let second = Style { foreground: Some(Color::Palette(10)), underline: true, ..first };
        let third = Style { foreground: Some(Color::Palette(7)), dim: true, ..second };
        assert_eq!(spans.as_deref(), Some(&[span(0, first), span(1, second), span(2, third)][..]));
    }

    #[test]
    fn the_style_holds_into_the_next_line_of_the_source_until_a_reset() {
        let mut pen = Pen::default();
        let (_, spans) = pen.draw(b"\x1b[31mstart");
        assert_eq!(spans.as_deref(), Some(&[span(0, red())][..]));
        let (text, spans) = pen.draw(b"still red");
        assert_eq!((&*text, spans.as_deref()), ("still red", Some(&[span(0, red())][..])));
        let (_, spans) = pen.draw(b"\x1b[39mdefault");
        assert_eq!(spans, None, "a line in the default color has no span");
        let (_, spans) = pen.draw(b"plain");
        assert_eq!(spans, None);
    }

    #[test]
    fn other_sequences_and_control_bytes_go_and_a_sync_start_loses_no_text() {
        let mut pen = Pen::default();
        let (text, spans) = pen.draw(b"\x1b]0;title\x07pro\rgress\x1b[2K\x1b[1A 100%\x08\x07");
        assert_eq!((&*text, spans), ("progress 100%", None));
        let (text, _) = pen.draw(b"\x1b[?2026hkept \x1b[32mtext");
        assert_eq!(&*text, "kept text");
        let (text, _) = pen.draw(b"cut \x1b[3");
        assert_eq!(&*text, "cut ", "a sequence that the line cut has no text");
    }
}
