//! The diff of one text value before a save (research 08 section 8.3): the lines that the
//! change removes and adds, with two lines of context around them.
//!
//! The diff compares whole lines with their line ends, so a line feed that an editor adds
//! or removes at the end, and a carriage return, show as a change: the save would send
//! them. The lines between the first and the last change all show as removed and added.
//! That costs one pass over each value, also for a value of a megabyte, and a value of a
//! Secret or a ConfigMap is rarely a long text with changes far apart.

use crate::format;

/// The lines of context before and after the changed lines.
const CONTEXT: usize = 2;
/// The most lines of the diff of one key. The dialog lays out each line in each frame.
const MOST_LINES: usize = 400;

/// What a line of a diff is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mark {
    /// A line of the two values.
    Same,
    /// A line of the value of the read only.
    Removed,
    /// A line of the new value only.
    Added,
    /// Lines that do not show; the text gives their count.
    Skipped,
    /// The line above is the end of its value, with no line feed after it.
    NoLineFeed,
}

/// One line of a diff.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Line {
    pub(super) mark: Mark,
    pub(super) text: String,
}

/// The diff of two texts. Two equal texts give no line.
pub(super) fn lines(old: &str, new: &str) -> Vec<Line> {
    let old: Vec<&str> = old.split_inclusive('\n').collect();
    let new: Vec<&str> = new.split_inclusive('\n').collect();
    let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let rest = old.len().min(new.len()) - prefix;
    let suffix = old.iter().rev().zip(new.iter().rev()).take(rest).take_while(|(a, b)| a == b).count();
    if prefix == old.len() && prefix == new.len() {
        return Vec::new();
    }
    let mut out = Out::default();
    let start = prefix.saturating_sub(CONTEXT);
    out.skip(start);
    out.add(Mark::Same, new.iter().skip(start).take(prefix - start));
    out.add(Mark::Removed, old.iter().skip(prefix).take(old.len() - suffix - prefix));
    out.add(Mark::Added, new.iter().skip(prefix).take(new.len() - suffix - prefix));
    let after = new.len() - suffix;
    let shown = suffix.min(CONTEXT);
    out.add(Mark::Same, new.iter().skip(after).take(shown));
    out.skip(suffix - shown);
    out.done()
}

#[derive(Default)]
struct Out {
    lines: Vec<Line>,
    /// The count of changed or context lines past the limit.
    over: usize,
}

impl Out {
    /// A mark for lines with no change that do not show.
    fn skip(&mut self, count: usize) {
        if count > 0 {
            let text = format::count(count as u64, "line");
            self.lines.push(Line { mark: Mark::Skipped, text: format!("{text} with no change") });
        }
    }

    fn add<'a>(&mut self, mark: Mark, lines: impl Iterator<Item = &'a &'a str>) {
        for line in lines {
            if self.lines.len() >= MOST_LINES {
                self.over += 1;
                continue;
            }
            let body = line.strip_suffix('\n');
            let text = body.unwrap_or(line).replace('\r', "\\r");
            self.lines.push(Line { mark, text });
            if body.is_none() {
                self.lines.push(Line { mark: Mark::NoLineFeed, text: "no line feed at the end".to_owned() });
            }
        }
    }

    fn done(mut self) -> Vec<Line> {
        if self.over > 0 {
            let noun = if self.over == 1 { "line" } else { "lines" };
            let text = format!("{} more {noun}", format::thousands(self.over as u64));
            self.lines.push(Line { mark: Mark::Skipped, text });
        }
        self.lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shown(old: &str, new: &str) -> Vec<String> {
        let sign = |mark| match mark {
            Mark::Same => " ",
            Mark::Removed => "-",
            Mark::Added => "+",
            Mark::Skipped => "~",
            Mark::NoLineFeed => "\\",
        };
        lines(old, new).into_iter().map(|line| format!("{}{}", sign(line.mark), line.text)).collect()
    }

    #[test]
    fn a_change_shows_with_two_lines_of_context_and_the_other_lines_as_a_count() {
        let old = "a\nb\nc\nd\ne\nf\ng\n";
        let new = "a\nb\nc\nD\ne\nf\ng\n";
        assert_eq!(
            shown(old, new),
            ["~1 line with no change", " b", " c", "-d", "+D", " e", " f", "~1 line with no change"]
        );
        assert_eq!(shown(old, old), Vec::<String>::new(), "no change, no line");
        assert_eq!(shown("", "x=1\ny=2\n"), ["+x=1", "+y=2"], "a new key");
        assert_eq!(shown("x=1\n", ""), ["-x=1"], "a key that goes away");
    }

    #[test]
    fn a_line_feed_and_a_carriage_return_that_an_editor_changes_show_as_a_change() {
        assert_eq!(shown("user\npass", "user\npass\n"), [" user", "-pass", "\\no line feed at the end", "+pass"]);
        assert_eq!(shown("a\r\nb\n", "a\nb\n"), ["-a\\r", "+a", " b"]);
        assert_eq!(shown("one", "two"), ["-one", "\\no line feed at the end", "+two", "\\no line feed at the end"]);
    }

    #[test]
    fn a_long_diff_ends_with_the_count_of_the_lines_that_do_not_show() {
        let old: String = (0..1000).map(|line| format!("{line}\n")).collect();
        let found = lines(&old, "");
        assert_eq!(found.len(), MOST_LINES + 1);
        assert_eq!(found.last().map(|line| line.text.as_str()), Some("600 more lines"));
    }
}
