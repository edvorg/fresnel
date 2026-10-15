//! The lines of one view: a ring with a limit of lines and a limit of bytes (design rule 12).
//!
//! This is the one module that uses the vector of `imbl`. A copy of the vector shares each
//! part that did not change, so a view gets the lines of one moment as a copy of a pointer,
//! and the next line changes few nodes. Each line has a number that stays when older lines
//! leave the ring: a scroll position, a selection and a search hit hold a number.

use std::fmt;

use imbl::Vector;

use super::line::LogLine;

/// The lines of a view at one moment. A clone copies a pointer, so a worker thread of the
/// view can take the lines for a search or a copy.
#[derive(Clone, Default)]
pub struct Lines {
    lines: Vector<LogLine>,
    /// The number of the first line.
    first: u64,
}

impl Lines {
    /// The number of the first line. The ring drops the oldest lines first, so this number
    /// grows.
    pub fn first(&self) -> u64 {
        self.first
    }

    /// The number after the last line: the number of the next line.
    pub fn end(&self) -> u64 {
        self.first + self.lines.len() as u64
    }

    /// The count of lines.
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// True with no line.
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// The line with this number. `None` for a line that left the ring or did not come yet.
    pub fn get(&self, number: u64) -> Option<&LogLine> {
        let index = number.checked_sub(self.first)?;
        self.lines.get(usize::try_from(index).ok()?)
    }

    /// The lines from the number `from` to the number before `to`, with their numbers. The
    /// part outside the ring has no line.
    pub fn range(&self, from: u64, to: u64) -> impl Iterator<Item = (u64, &LogLine)> {
        let start = from.clamp(self.first, self.end());
        let stop = to.clamp(start, self.end());
        let index = |number: u64| usize::try_from(number - self.first).unwrap_or(usize::MAX).min(self.lines.len());
        let lines = self.lines.focus().narrow(index(start)..index(stop));
        (start..).zip(lines)
    }
}

impl fmt::Debug for Lines {
    /// The numbers only: a view can have 200,000 lines.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Lines").field("first", &self.first).field("end", &self.end()).finish()
    }
}

impl PartialEq for Lines {
    /// Two copies of one ring at one moment are equal: the ring changes with each line.
    fn eq(&self, other: &Self) -> bool {
        self.first == other.first && self.lines.ptr_eq(&other.lines)
    }
}

/// The limits of the ring of one view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RingLimits {
    /// The most lines.
    pub lines: usize,
    /// The most bytes of the lines in memory, with their text and styles.
    pub bytes: usize,
}

/// The ring of one view: the oldest line leaves when a limit is full.
pub(super) struct Ring {
    lines: Lines,
    bytes: usize,
    limits: RingLimits,
    /// The count of lines that left the ring.
    dropped: u64,
}

impl Ring {
    pub(super) fn new(limits: RingLimits) -> Self {
        Self { lines: Lines::default(), bytes: 0, limits, dropped: 0 }
    }

    /// Adds a line at the end. Older lines leave until the ring is inside its limits; the
    /// new line stays, also when it is larger than the limit of bytes alone.
    pub(super) fn push(&mut self, line: LogLine) {
        self.bytes += line.weight();
        self.lines.lines.push_back(line);
        while self.lines.len() > 1 && (self.lines.len() > self.limits.lines || self.bytes > self.limits.bytes) {
            let Some(old) = self.lines.lines.pop_front() else {
                break;
            };
            self.bytes = self.bytes.saturating_sub(old.weight());
            self.lines.first += 1;
            self.dropped += 1;
        }
    }

    /// The lines of this moment.
    pub(super) fn lines(&self) -> Lines {
        self.lines.clone()
    }

    /// The count of lines that left the ring.
    pub(super) fn dropped(&self) -> u64 {
        self.dropped
    }

    /// The bytes of the lines in the ring.
    pub(super) fn bytes(&self) -> usize {
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logs::line::LineKind;

    fn line(number: u64) -> LogLine {
        let text = format!("line {number:07} of the test with some words after it");
        LogLine {
            time: i64::try_from(number).unwrap_or_default(),
            source: 0,
            text: text.into(),
            spans: None,
            kind: LineKind::Log,
            late: false,
            cut: false,
        }
    }

    #[test]
    fn one_million_lines_leave_the_ring_at_its_limit_of_lines_and_each_number_stays() {
        let mut ring = Ring::new(RingLimits { lines: 200_000, bytes: usize::MAX });
        let copy_before = ring.lines();
        for number in 0..1_000_000 {
            ring.push(line(number));
        }
        let lines = ring.lines();
        assert_eq!((lines.len(), lines.first(), lines.end(), ring.dropped()), (200_000, 800_000, 1_000_000, 800_000));
        assert_eq!(ring.bytes(), (0..200_000).map(|_| line(0).weight()).sum::<usize>());
        assert_eq!(lines.get(800_000).map(|line| line.time), Some(800_000));
        assert_eq!(lines.get(999_999).map(|line| line.time), Some(999_999));
        assert_eq!((lines.get(799_999), lines.get(1_000_000)), (None, None), "outside the ring");
        assert!(copy_before.is_empty(), "a copy of an earlier moment does not change");
    }

    #[test]
    fn the_limit_of_bytes_drops_old_lines_and_a_line_larger_than_the_limit_stays_alone() {
        let weight = line(0).weight();
        let mut ring = Ring::new(RingLimits { lines: 1000, bytes: 3 * weight });
        for number in 0..10 {
            ring.push(line(number));
        }
        let numbers: Vec<i64> = ring.lines().range(0, 100).map(|(_, line)| line.time).collect();
        assert_eq!((numbers, ring.dropped(), ring.bytes()), (vec![7, 8, 9], 7, 3 * weight));

        let mut huge = line(10);
        huge.text = "x".repeat(10 * weight).into();
        ring.push(huge);
        assert_eq!((ring.lines().len(), ring.lines().first(), ring.dropped()), (1, 10, 10));
    }

    #[test]
    fn a_range_gives_the_part_inside_the_ring_with_the_numbers() {
        let mut ring = Ring::new(RingLimits { lines: 5, bytes: usize::MAX });
        for number in 0..8 {
            ring.push(line(number));
        }
        let lines = ring.lines();
        let part = |from, to| lines.range(from, to).map(|(number, line)| (number, line.time)).collect::<Vec<_>>();
        assert_eq!(part(0, 5), [(3, 3), (4, 4)]);
        assert_eq!(part(6, 100), [(6, 6), (7, 7)]);
        assert_eq!((part(7, 2), part(100, 200)), (vec![], vec![]));
        assert_eq!(lines.len(), 5);
        let copy = ring.lines();
        assert_eq!(lines, copy);
        ring.push(line(8));
        assert_ne!(lines, ring.lines(), "a new line gives a new moment");
    }
}
