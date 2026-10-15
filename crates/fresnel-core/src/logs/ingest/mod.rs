//! From the data of the streams of one source to the lines of a view (research 05, section 5.3).
//!
//! The data is raw bytes. It becomes lines with a length limit (`split`), each line gives
//! its time (`stamp`), a line that a new stream sends again goes (`resume`), and the text
//! loses its escape sequences and gets its styles (`sgr`). A line that the parser could not
//! read as UTF-8 keeps each other character.
//!
//! The kubelet writes each line of a log with its time. A line with no time is a text of the
//! kubelet itself: when it cannot read the log, for example of an instance that the runtime
//! removed, it writes "unable to retrieve container logs for …" into the answer, with HTTP
//! 200 (seen on k3s 1.33). A stream with only such text is a refusal, not a log.

mod resume;
mod sgr;
mod split;
mod stamp;

use std::sync::Arc;

use k8s_openapi::jiff::Timestamp;

#[cfg(test)]
pub(crate) use stamp::NANOS;
pub(crate) use stamp::second_of;

use super::line::{LineKind, LogLine};
use resume::{Replay, Seen};
use sgr::Pen;
use split::Splitter;

/// The lines of all streams of one source, one stream after the other.
pub(super) struct Ingest {
    splitter: Splitter,
    lines: Liner,
}

/// What turns one raw line into a line of the view.
struct Liner {
    source: u32,
    pen: Pen,
    seen: Seen,
    replay: Replay,
    /// The count of lines that a new stream sent again.
    repeated: u64,
    /// True after a line with a time in the current stream.
    timed: bool,
    /// The lines with no time of the current stream before its first line with a time.
    untimed: Vec<Arc<str>>,
}

/// The most lines with no time that a stream keeps before its first line with a time.
const UNTIMED_LIMIT: usize = 8;

impl Ingest {
    /// The lines of the source with this index; a line keeps `line_limit` bytes at most.
    pub(super) fn new(source: u32, line_limit: usize) -> Self {
        let (pen, seen, replay) = (Pen::default(), Seen::default(), Replay::default());
        let lines = Liner { source, pen, seen, replay, repeated: 0, timed: false, untimed: Vec::new() };
        Self { splitter: Splitter::new(line_limit), lines }
    }

    /// A new stream starts: the start of a line of the last stream goes, and the lines of
    /// the latest second come again.
    pub(super) fn restart(&mut self) {
        self.splitter.drop_partial();
        let lines = &mut self.lines;
        (lines.replay, lines.timed) = (Replay::after(&lines.seen), false);
        lines.untimed.clear();
    }

    /// Adds the full lines of a piece of data to `out`.
    pub(super) fn feed(&mut self, data: &[u8], out: &mut Vec<LogLine>) {
        let lines = &mut self.lines;
        self.splitter.feed(data, &mut |raw, cut| lines.line(raw, cut, out));
    }

    /// The stream ended: its last line with no line end is a line too. Gives the text of
    /// the kubelet when the stream had no line with a time: it is a refusal then.
    pub(super) fn finish(&mut self, out: &mut Vec<LogLine>) -> Option<String> {
        let lines = &mut self.lines;
        self.splitter.finish(&mut |raw, cut| lines.line(raw, cut, out));
        let texts = std::mem::take(&mut lines.untimed);
        (!lines.timed && !texts.is_empty()).then(|| texts.join("; "))
    }

    /// The start of the next stream: the full second of the latest line. `None` before
    /// the first line.
    pub(super) fn start(&self) -> Option<Timestamp> {
        self.lines.seen.start().map(second_of)
    }

    /// The time of the latest line. `None` before the first line.
    pub(super) fn latest(&self) -> Option<i64> {
        self.lines.seen.latest()
    }

    /// The count of lines that a new stream sent again, and that the view has one time.
    pub(super) fn repeated(&self) -> u64 {
        self.lines.repeated
    }
}

impl Liner {
    fn line(&mut self, raw: &[u8], cut: bool, out: &mut Vec<LogLine>) {
        let Some((time, text)) = stamp::split(raw) else {
            self.untimed(raw, out);
            return;
        };
        if !self.timed {
            self.timed = true;
            for text in std::mem::take(&mut self.untimed) {
                out.push(self.kubelet(&text));
            }
        }
        if self.replay.repeats(time) {
            self.repeated += 1;
            return;
        }
        self.seen.note(time);
        let text = text.strip_suffix(b"\r").unwrap_or(text);
        let text = if cut { whole_characters(text) } else { text };
        let (text, spans) = self.pen.draw(text);
        out.push(LogLine { time, source: self.source, text, spans, kind: LineKind::Log, late: false, cut });
    }

    /// A line with no time: a text of the kubelet. Before the first line with a time, it
    /// waits, because it can be the only text of a refusal.
    fn untimed(&mut self, raw: &[u8], out: &mut Vec<LogLine>) {
        let text: Arc<str> = String::from_utf8_lossy(raw.strip_suffix(b"\r").unwrap_or(raw)).into();
        if self.timed {
            out.push(self.kubelet(&text));
        } else if self.untimed.len() < UNTIMED_LIMIT {
            self.untimed.push(text);
        }
    }

    /// A marker with a text of the kubelet, at the time of the latest line.
    fn kubelet(&self, text: &str) -> LogLine {
        LogLine::marker(self.seen.latest().unwrap_or_default(), self.source, format!("the kubelet wrote: {text}"))
    }
}

/// The text of a line that the limit cut, with no part of a character at its end.
fn whole_characters(text: &[u8]) -> &[u8] {
    match std::str::from_utf8(text) {
        Err(error) if error.error_len().is_none() => text.get(..error.valid_up_to()).unwrap_or(text),
        Ok(_) | Err(_) => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(lines: &[LogLine]) -> Vec<String> {
        let text =
            |line: &LogLine| format!("{} {}{}", line.time % NANOS, line.text, if line.cut { " (cut)" } else { "" });
        lines.iter().map(text).collect()
    }

    #[test]
    fn a_new_stream_after_a_break_gives_each_line_one_time() {
        let mut ingest = Ingest::new(3, 1024);
        let mut lines = Vec::new();
        ingest
            .feed(b"2026-10-12T20:00:00.1Z one\n2026-10-12T20:00:01.1Z two\r\n2026-10-12T20:00:01.2Z thr", &mut lines);
        assert_eq!(texts(&lines), ["100000000 one", "100000000 two"]);
        assert_eq!(ingest.start().map(|start| start.to_string()).as_deref(), Some("2026-10-12T20:00:01Z"));

        // The stream broke in the middle of a line. The next one starts at the second of
        // the latest line and sends its lines again.
        ingest.restart();
        lines.clear();
        let again = b"2026-10-12T20:00:01.1Z two\n2026-10-12T20:00:01.2Z three\n2026-10-12T20:00:02Z \x1b[1mfour";
        ingest.feed(again, &mut lines);
        assert_eq!(ingest.finish(&mut lines), None);
        assert_eq!(texts(&lines), ["200000000 three", "0 four"]);
        assert_eq!((ingest.repeated(), lines.iter().all(|line| line.source == 3)), (1, true));
        assert!(lines.last().and_then(|line| line.spans.as_ref()).is_some(), "the bold text has its style");
    }

    #[test]
    fn a_cut_line_ends_with_a_whole_character_and_a_line_with_no_time_is_a_text_of_the_kubelet() {
        let mut ingest = Ingest::new(0, 26);
        let mut lines = Vec::new();
        ingest.feed("2026-10-12T20:00:00.1Z äöü\nno time\n".as_bytes(), &mut lines);
        assert_eq!(texts(&lines), ["100000000 ä (cut)", "100000000 the kubelet wrote: no time"]);
        assert_eq!(lines[1].kind, LineKind::Marker);
        assert_eq!(ingest.finish(&mut lines), None, "the stream had lines with a time");
    }

    /// Seen on k3s: the log of an instance that the runtime removed.
    #[test]
    fn a_stream_with_only_a_text_of_the_kubelet_is_a_refusal_and_gives_no_line() {
        let mut ingest = Ingest::new(0, 1024);
        let mut lines = Vec::new();
        ingest.feed(b"unable to retrieve container logs for containerd://2876\n", &mut lines);
        let refusal = ingest.finish(&mut lines);
        assert_eq!(refusal.as_deref(), Some("unable to retrieve container logs for containerd://2876"));
        assert_eq!((lines.len(), ingest.latest()), (0, None));

        ingest.restart();
        ingest.feed(b"a note\n2026-10-12T20:00:00.1Z a line\n", &mut lines);
        assert_eq!(texts(&lines), ["0 the kubelet wrote: a note", "100000000 a line"]);
        assert_eq!(ingest.finish(&mut lines), None);
    }
}
