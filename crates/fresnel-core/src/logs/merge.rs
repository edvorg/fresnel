//! The sequence of the lines of many sources in one view (research 05, section 5.4).
//!
//! Each source has a queue. The line with the lowest time goes into the view when each
//! source that can still send has a line, when a line waited for the hold time (200 to
//! 300 ms), or when the queues hold more than their limit of bytes. Thus a slow source holds
//! the view for that time at most, and a fast one fills no memory meanwhile (design rule 12).
//! A line that arrives after a later line went into the view goes to the end with the mark
//! "late": the view never puts a line into its middle, so each number of a line stays.

use std::collections::VecDeque;
use std::time::Duration;

use tokio::time::Instant;

use super::line::{LineKind, LogLine};

/// The queue of one source.
#[derive(Default)]
struct Queue {
    /// The lines with the time of their arrival, in the sequence of their stream.
    lines: VecDeque<(LogLine, Instant)>,
    /// True while the source can send more lines: its stream opens or is open.
    open: bool,
}

impl Queue {
    /// Frees the room of a source that cannot send and has no line. A queue keeps the room of
    /// its largest burst, and a view of a workload whose pods come and go has a queue for each
    /// pod that it had: each one would keep that room until the view closes.
    fn release(&mut self) {
        if !self.open && self.lines.is_empty() {
            self.lines = VecDeque::new();
        }
    }
}

/// The lines of all sources of a view, before they go into its ring.
pub(super) struct Merge {
    queues: Vec<Queue>,
    hold: Duration,
    /// The bytes of the lines in the queues, as the ring counts them (`LogLine::weight`).
    held: usize,
    /// The most bytes in the queues: above it, the oldest line goes with no wait.
    limit: usize,
    /// The time of the latest line that went into the view.
    latest: Option<i64>,
}

impl Merge {
    pub(super) fn new(hold: Duration, limit: usize) -> Self {
        Self { queues: Vec::new(), hold, held: 0, limit, latest: None }
    }

    /// Tells if the source with this index can send more lines. A new index adds a source.
    pub(super) fn set_open(&mut self, source: u32, open: bool) {
        if let Some(queue) = self.queue(source) {
            queue.open = open;
            queue.release();
        }
    }

    /// Adds the lines of one source that arrived at `now`.
    pub(super) fn push(&mut self, source: u32, lines: Vec<LogLine>, now: Instant) {
        let Some(queue) = self.queue(source) else {
            return;
        };
        let weight: usize = lines.iter().map(LogLine::weight).sum();
        queue.lines.extend(lines.into_iter().map(|line| (line, now)));
        self.held += weight;
    }

    /// Gives each line that can go into the view now to `out`, in its sequence.
    pub(super) fn drain(&mut self, now: Instant, out: &mut impl FnMut(LogLine)) {
        loop {
            let heads = self.queues.iter().enumerate().filter_map(|(index, queue)| Some((index, queue.lines.front()?)));
            let Some((first, _)) = heads.min_by_key(|(index, (line, _))| (line.time, *index)) else {
                return;
            };
            let each_has_one = self.queues.iter().all(|queue| !queue.open || !queue.lines.is_empty());
            let waited = self.oldest().is_some_and(|arrived| arrived + self.hold <= now);
            if !each_has_one && !waited && self.held <= self.limit {
                return;
            }
            let Some(queue) = self.queues.get_mut(first) else {
                return;
            };
            let Some((mut line, _)) = queue.lines.pop_front() else {
                return;
            };
            queue.release();
            self.held = self.held.saturating_sub(line.weight());
            line.late = line.kind == LineKind::Log && self.latest.is_some_and(|latest| line.time < latest);
            self.latest = Some(self.latest.map_or(line.time, |latest| latest.max(line.time)));
            out(line);
        }
    }

    /// The time when the oldest line that waits must go: the next drain after a pause.
    pub(super) fn deadline(&self) -> Option<Instant> {
        self.oldest().map(|arrived| arrived + self.hold)
    }

    /// The arrival of the oldest line that waits. The first line of a queue is its oldest.
    fn oldest(&self) -> Option<Instant> {
        self.queues.iter().filter_map(|queue| queue.lines.front().map(|(_, arrived)| *arrived)).min()
    }

    fn queue(&mut self, source: u32) -> Option<&mut Queue> {
        let index = usize::try_from(source).ok()?;
        if self.queues.len() <= index {
            self.queues.resize_with(index + 1, Queue::default);
        }
        self.queues.get_mut(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOLD: Duration = Duration::from_millis(250);

    fn line(source: u32, time: i64) -> LogLine {
        LogLine {
            time,
            source,
            text: format!("{source}@{time}").into(),
            spans: None,
            kind: LineKind::Log,
            late: false,
            cut: false,
        }
    }

    fn drained(merge: &mut Merge, now: Instant) -> Vec<String> {
        let mut out = Vec::new();
        merge.drain(now, &mut |line| out.push(format!("{}{}", line.text, if line.late { " late" } else { "" })));
        out
    }

    #[test]
    fn the_lines_of_open_sources_go_in_the_sequence_of_time_when_each_source_has_one() {
        let start = Instant::now();
        let mut merge = Merge::new(HOLD, usize::MAX);
        merge.set_open(0, true);
        merge.set_open(1, true);
        merge.push(0, vec![line(0, 10), line(0, 30)], start);
        assert_eq!(drained(&mut merge, start), Vec::<String>::new(), "the second source has no line yet");
        assert_eq!(merge.deadline(), Some(start + HOLD));
        merge.push(1, vec![line(1, 20), line(1, 25)], start);
        assert_eq!(drained(&mut merge, start), ["0@10", "1@20", "1@25"]);
        merge.set_open(1, false);
        assert_eq!(drained(&mut merge, start), ["0@30"], "an ended source holds nobody");
        assert_eq!(merge.deadline(), None);
    }

    #[test]
    fn a_silent_source_holds_the_view_for_the_hold_time_and_its_later_line_is_late() {
        let start = Instant::now();
        let mut merge = Merge::new(HOLD, usize::MAX);
        merge.set_open(0, true);
        merge.set_open(1, true);
        merge.push(0, vec![line(0, 10), line(0, 30)], start);
        let soon = start + HOLD - Duration::from_millis(1);
        assert_eq!(drained(&mut merge, soon), Vec::<String>::new());
        assert_eq!(drained(&mut merge, start + HOLD), ["0@10", "0@30"]);
        merge.push(1, vec![line(1, 20), line(1, 40)], start + HOLD);
        merge.push(0, vec![line(0, 50)], start + HOLD);
        assert_eq!(drained(&mut merge, start + HOLD), ["1@20 late", "1@40"], "0@50 waits for a later line of 1");
        assert_eq!(drained(&mut merge, start + 2 * HOLD), ["0@50"]);
    }

    /// A silent source of a workload, and a fast one that sends its full log at once.
    #[test]
    fn the_queues_hold_their_limit_of_bytes_and_no_more_while_a_source_is_silent() {
        let start = Instant::now();
        let weight = line(0, 100).weight();
        let mut merge = Merge::new(HOLD, 10 * weight);
        merge.set_open(0, true);
        merge.set_open(1, true);
        merge.push(0, (100..200).map(|time| line(0, time)).collect(), start);
        let first = drained(&mut merge, start);
        assert_eq!((first.len(), first.last().map(String::as_str)), (90, Some("0@189")), "the oldest lines go");
        assert_eq!(merge.held, 10 * weight);
        merge.push(1, vec![line(1, 150)], start);
        assert_eq!(drained(&mut merge, start), ["1@150 late"], "a line of the silent source is late");
        assert_eq!((drained(&mut merge, start + HOLD).len(), merge.held), (10, 0));
    }

    /// A pod that sends a burst while an other one is silent fills its queue. When its source
    /// ends, the queue frees that room: a view keeps a queue for each pod that it had.
    #[test]
    fn the_queue_of_a_source_that_ended_frees_the_room_of_its_burst() {
        let start = Instant::now();
        let mut merge = Merge::new(HOLD, usize::MAX);
        merge.set_open(0, true);
        merge.set_open(1, true);
        merge.push(0, (0..1000).map(|time| line(0, time)).collect(), start);
        assert_eq!(drained(&mut merge, start + HOLD).len(), 1000);
        assert!(merge.queues[0].lines.capacity() >= 1000, "an open source keeps its room");
        merge.set_open(0, false);
        assert_eq!(merge.queues[0].lines.capacity(), 0, "a source that ended with no line");

        merge.push(1, vec![line(1, 2000), line(1, 2001)], start + HOLD);
        merge.set_open(1, false);
        assert!(merge.queues[1].lines.capacity() >= 2, "its lines wait for the drain");
        assert_eq!(drained(&mut merge, start + HOLD), ["1@2000", "1@2001"]);
        assert_eq!(merge.queues[1].lines.capacity(), 0, "the drain of its last line");
    }

    #[test]
    fn one_source_goes_at_once_and_a_marker_is_never_late() {
        let start = Instant::now();
        let mut merge = Merge::new(HOLD, usize::MAX);
        merge.set_open(3, true);
        merge.push(3, vec![line(3, 10), line(3, 5)], start);
        let mut marker = line(3, 7);
        marker.kind = LineKind::Marker;
        merge.push(3, vec![marker], start);
        assert_eq!(drained(&mut merge, start), ["3@10", "3@5 late", "3@7"], "a stream keeps its sequence");
    }
}
