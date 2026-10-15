//! The lines of a new stream that the view has already (plan section 12.7).
//!
//! A new stream of a source starts at the full second of its latest line (see
//! `stamp::second_of`), so it sends the lines of that second again. [`Seen`] keeps the
//! times of the lines of that second in their sequence. [`Replay`] drops the lines of a new
//! stream while they repeat that sequence, and the first other line ends the replay. Thus a
//! line that the view does not have is never dropped, also with two lines of one time.

use super::stamp::NANOS;

/// The most times of one second that a source keeps. A source with more lines in one second
/// shows some of them two times after a new stream, and it loses none.
const SECOND_LIMIT: usize = 100_000;

/// The lines of the latest second of a source.
#[derive(Default)]
pub(super) struct Seen {
    /// The latest second, in seconds since 1970.
    second: Option<i64>,
    /// The time of each line of that second, in the sequence of their arrival.
    times: Vec<i64>,
}

impl Seen {
    /// Notes a line that went into the view.
    pub(super) fn note(&mut self, time: i64) {
        let second = time.div_euclid(NANOS);
        match self.second {
            // The next stream starts after this line, so it does not send it again.
            Some(latest) if second < latest => {}
            Some(latest) if second == latest => {
                if self.times.len() < SECOND_LIMIT {
                    self.times.push(time);
                }
            }
            Some(_) | None => {
                self.second = Some(second);
                self.times.clear();
                self.times.push(time);
            }
        }
    }

    /// The time of the latest line, in nanoseconds since 1970. `None` before a line.
    pub(super) fn latest(&self) -> Option<i64> {
        self.times.iter().max().copied()
    }

    /// The start of the next stream: the full second of the latest line.
    pub(super) fn start(&self) -> Option<i64> {
        self.second.map(|second| second * NANOS)
    }
}

/// The start of a new stream, which repeats the lines of the latest second.
#[derive(Default)]
pub(super) struct Replay {
    /// The index in `Seen::times` of the next line that the stream repeats.
    next: usize,
    open: bool,
}

impl Replay {
    /// The replay of a new stream from the second of `seen`.
    pub(super) fn after(seen: &Seen) -> Self {
        Self { next: 0, open: seen.second.is_some() }
    }

    /// True for a line that the view has already. The first other line ends the replay.
    pub(super) fn repeats(&mut self, seen: &Seen, time: i64) -> bool {
        if self.open && seen.times.get(self.next) == Some(&time) {
            self.next += 1;
            return true;
        }
        self.open = false;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The times of the lines of the new stream that go into the view.
    fn kept(seen: &mut Seen, stream: &[i64]) -> Vec<i64> {
        let mut replay = Replay::after(seen);
        let mut kept = Vec::new();
        for time in stream {
            if !replay.repeats(seen, *time) {
                seen.note(*time);
                kept.push(*time);
            }
        }
        kept
    }

    fn seen(times: &[i64]) -> Seen {
        let mut seen = Seen::default();
        for time in times {
            seen.note(*time);
        }
        seen
    }

    const S: i64 = NANOS;

    #[test]
    fn the_lines_of_the_last_second_come_again_and_go_and_the_new_lines_stay() {
        let mut seen = seen(&[4 * S + 1, 5 * S + 2, 5 * S + 7, 5 * S + 7]);
        assert_eq!((seen.start(), seen.latest()), (Some(5 * S), Some(5 * S + 7)));
        let stream = [5 * S + 2, 5 * S + 7, 5 * S + 7, 5 * S + 7, 5 * S + 9, 6 * S];
        assert_eq!(kept(&mut seen, &stream), [5 * S + 7, 5 * S + 9, 6 * S], "a third line of the same time is new");
        assert_eq!(seen.start(), Some(6 * S));
    }

    #[test]
    fn a_stream_that_differs_from_the_lines_keeps_each_line_from_the_first_difference() {
        let mut seen = seen(&[5 * S + 2, 5 * S + 3]);
        // The log rotated: the line of the time 5.2 s is gone.
        assert_eq!(kept(&mut seen, &[5 * S + 3, 5 * S + 4]), [5 * S + 3, 5 * S + 4]);
        let mut empty = Seen::default();
        assert_eq!(kept(&mut empty, &[7, 8]), [7, 8], "with no line before, each line is new");
    }

    #[test]
    fn a_line_of_an_earlier_second_after_a_later_one_is_not_in_the_replay() {
        // The runtime writes stdout and stderr into one file, so the sequence of the times
        // can go back. The server sends only the lines from the start second again.
        let mut seen = seen(&[5 * S + 9, 6 * S + 1, 5 * S + 95, 6 * S + 2]);
        assert_eq!(seen.latest(), Some(6 * S + 2));
        assert_eq!(kept(&mut seen, &[6 * S + 1, 6 * S + 2, 6 * S + 3]), [6 * S + 3]);
    }
}
