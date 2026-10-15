//! The cut of the data of a stream into lines, with a limit for one line.
//!
//! The data arrives in pieces, and a piece can end in the middle of a line. A program can
//! write a long text with no line end, so a line keeps its first `limit` bytes, and the rest
//! up to its line end goes (research 05, section 5.1).

/// The lines of one stream.
pub(super) struct Splitter {
    /// The start of a line whose end did not arrive.
    partial: Vec<u8>,
    /// True when the line of `partial` lost bytes at the limit.
    cut: bool,
    limit: usize,
}

impl Splitter {
    pub(super) fn new(limit: usize) -> Self {
        Self { partial: Vec::new(), cut: false, limit }
    }

    /// Gives each full line of the data to `line`, with no line end and with a flag for a
    /// line that lost bytes at the limit. The start of a last line waits for its end.
    pub(super) fn feed(&mut self, mut data: &[u8], line: &mut impl FnMut(&[u8], bool)) {
        while let Some(end) = data.iter().position(|byte| *byte == b'\n') {
            let (head, rest) = data.split_at(end);
            data = rest.get(1..).unwrap_or_default();
            if self.partial.is_empty() && !self.cut {
                // The full line is in this piece: no copy.
                let kept = head.get(..self.limit).unwrap_or(head);
                line(kept, kept.len() < head.len());
            } else {
                self.keep(head);
                line(&self.partial, self.cut);
                self.partial.clear();
                self.cut = false;
            }
        }
        self.keep(data);
    }

    /// The end of a stream: a last line with no line end is a line too. A container can
    /// write its last text with no line end.
    pub(super) fn finish(&mut self, line: &mut impl FnMut(&[u8], bool)) {
        if !self.partial.is_empty() || self.cut {
            line(&self.partial, self.cut);
        }
        self.drop_partial();
    }

    /// A stream that broke: the start of its last line goes, because the next stream sends
    /// the full line again.
    pub(super) fn drop_partial(&mut self) {
        self.partial = Vec::new();
        self.cut = false;
    }

    /// Keeps a part of a line up to the limit.
    fn keep(&mut self, part: &[u8]) {
        let room = self.limit.saturating_sub(self.partial.len());
        let kept = part.get(..room).unwrap_or(part);
        self.cut |= kept.len() < part.len();
        self.partial.extend_from_slice(kept);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The lines of the pieces, as text with a mark for a cut line, and the rest at the end.
    fn lines(limit: usize, pieces: &[&str], end: bool) -> Vec<String> {
        let mut splitter = Splitter::new(limit);
        let mut lines = Vec::new();
        let mut take = |line: &[u8], cut: bool| {
            lines.push(format!("{}{}", String::from_utf8_lossy(line), if cut { " (cut)" } else { "" }));
        };
        for piece in pieces {
            splitter.feed(piece.as_bytes(), &mut take);
        }
        if end {
            splitter.finish(&mut take);
        }
        lines
    }

    #[test]
    fn each_line_comes_one_time_however_the_data_arrives() {
        let pieces = ["one\ntw", "o\n", "", "\nthr", "e", "e\nfour"];
        assert_eq!(lines(100, &pieces, false), ["one", "two", "", "three"]);
        assert_eq!(lines(100, &pieces, true), ["one", "two", "", "three", "four"]);
        assert_eq!(lines(100, &["a\n"], true), ["a"], "no empty line after the last line end");
    }

    #[test]
    fn a_long_line_keeps_its_start_up_to_the_limit_and_the_next_line_is_whole() {
        assert_eq!(lines(4, &["abcdefgh\nij\n"], false), ["abcd (cut)", "ij"]);
        assert_eq!(lines(4, &["ab", "cdef", "gh", "\nij\n"], false), ["abcd (cut)", "ij"]);
        assert_eq!(lines(4, &["abcd\n"], false), ["abcd"], "a line of the limit is not cut");
        assert_eq!(lines(4, &["abcdefgh"], true), ["abcd (cut)"]);
    }

    #[test]
    fn a_stream_with_no_line_end_holds_the_limit_and_no_more() {
        let mut splitter = Splitter::new(1024);
        let piece = vec![b'x'; 64 * 1024];
        for _ in 0..100 {
            splitter.feed(&piece, &mut |_, _| panic!("no line end arrived"));
        }
        assert_eq!(splitter.partial.len(), 1024);
        splitter.drop_partial();
        assert_eq!(lines(4, &["ab"], false), Vec::<String>::new());
        assert_eq!((splitter.partial.capacity(), splitter.cut), (0, false), "a broken stream frees its part");
    }
}
