//! The search and the filter of a log view (plan section 12.7). A worker thread reads the
//! lines, so a search of 200,000 lines never holds a frame (design rule of section 7.3).
//!
//! The worker reads the lines in parts of `PART` lines. For each part, it gives the lines
//! that the filter keeps, and the lines with a hit of the search among them. A part for the
//! new lines follows when they come; a change of a text starts again at the first line, and
//! the worker drops the part that it reads for the old text. A line that leaves the ring
//! leaves the lists. A marker of the app always passes the filter: it tells of a gap.

use std::collections::VecDeque;
use std::sync::Arc;

use fresnel_core::logs::{LineKind, Lines};
use fresnel_core::net::task::{TaskName, Tasks};

use super::needle::Needle;
use crate::widgets::worker::{Cancel, RequestId, Wake, Worker};

/// The lines of one request to the worker: a part takes some milliseconds, so a new text
/// stops the old work soon, and the view tells the progress.
const PART: u64 = 20_000;
/// The lines between two looks at the stop of a part.
const STEP: usize = 1024;

/// One part of the lines for the worker.
#[derive(Clone)]
struct Scan {
    lines: Lines,
    from: u64,
    search: Option<Needle>,
    filter: Option<Needle>,
}

/// What the worker found in one part.
struct Scanned {
    from: u64,
    to: u64,
    kept: Vec<u64>,
    hits: Vec<u64>,
}

/// The search and the filter of one view.
#[derive(Default)]
pub(crate) struct Find {
    /// The texts of the two fields.
    pub(crate) search: String,
    pub(crate) filter: String,
    /// The needles of the lists below.
    needles: (Option<Needle>, Option<Needle>),
    /// A number that grows with each change of a needle and with each new view.
    query: u64,
    /// The lines that the filter keeps. Shared with a copy that reads them.
    kept: Arc<VecDeque<u64>>,
    /// The lines with a hit, among the kept ones.
    hits: VecDeque<u64>,
    /// The number of the first line that the worker did not read.
    done: u64,
    /// The start of the part that the worker reads now.
    asked: Option<u64>,
    /// The line of the hit that the user went to.
    current: Option<u64>,
    worker: Option<Worker<Scan, Scanned>>,
}

impl Find {
    /// Follows the texts of the fields and the lines of this frame: takes a part from the
    /// worker and asks for the next one.
    pub(crate) fn update(&mut self, lines: &Lines, tasks: &Tasks, wake: &Wake) {
        let needles = (Needle::new(&self.search), Needle::new(&self.filter));
        // A view that started again numbers its lines from 0.
        if needles != self.needles || lines.end() < self.done {
            self.needles = needles;
            self.restart();
        }
        if self.needles == (None, None) {
            return;
        }
        let worker = self.worker.get_or_insert_with(|| {
            Worker::start(tasks, TaskName::new("log search"), Arc::clone(wake), |scan: &Scan, cancel: &Cancel<'_>| {
                read(scan, cancel)
            })
        });
        if let Some(done) = worker.take() {
            if done.id.query == self.query && Some(done.output.from) == self.asked {
                let Scanned { to, kept, hits, .. } = done.output;
                Arc::make_mut(&mut self.kept).extend(kept);
                self.hits.extend(hits);
                (self.done, self.asked) = (to, None);
            } else {
                worker.discard(done.output);
            }
        }
        let first = lines.first();
        trim(Arc::make_mut(&mut self.kept), first);
        trim(&mut self.hits, first);
        if self.asked.is_none() && self.done < lines.end() {
            let from = self.done.max(first);
            let (search, filter) = self.needles.clone();
            worker.submit(
                RequestId { query: self.query, data: from },
                Scan { lines: lines.clone(), from, search, filter },
            );
            self.asked = Some(from);
        }
    }

    /// Starts again from the first line: a new text or a new view.
    pub(crate) fn restart(&mut self) {
        self.query += 1;
        (self.kept, self.hits, self.done, self.asked, self.current) = (Arc::default(), VecDeque::new(), 0, None, None);
    }

    /// The lines that the filter keeps. `None` with no filter.
    pub(crate) fn kept(&self) -> Option<&VecDeque<u64>> {
        self.needles.1.is_some().then_some(&*self.kept)
    }

    /// The same, for a copy on its worker thread.
    pub(crate) fn kept_shared(&self) -> Option<Arc<VecDeque<u64>>> {
        self.needles.1.is_some().then(|| Arc::clone(&self.kept))
    }

    /// The needle of the search, for the marks of its hits.
    pub(crate) fn search(&self) -> Option<&Needle> {
        self.needles.0.as_ref()
    }

    pub(crate) fn current(&self) -> Option<u64> {
        self.current.filter(|current| self.hits.binary_search(current).is_ok())
    }

    /// Goes to the next hit after the current one, or after the line `from` with none; after
    /// the last hit comes the first one. `forward` false goes back. Gives its line.
    pub(crate) fn go(&mut self, forward: bool, from: u64) -> Option<u64> {
        let at = self.current().unwrap_or(from);
        let next = if forward {
            self.hits.iter().find(|hit| **hit > at).or_else(|| self.hits.front())
        } else {
            self.hits.iter().rev().find(|hit| **hit < at).or_else(|| self.hits.back())
        };
        self.current = next.copied();
        self.current
    }

    /// "3 of 120 lines", "120 lines", "no line": the hits of the search, with "so far" while
    /// the worker reads. `None` with no search.
    pub(crate) fn count(&self, lines: &Lines) -> Option<String> {
        self.needles.0.as_ref()?;
        let total = crate::format::count(self.hits.len() as u64, "line");
        let text = match self.current().and_then(|current| self.hits.binary_search(&current).ok()) {
            Some(index) => format!("{} of {total}", crate::format::thousands(index as u64 + 1)),
            None if self.hits.is_empty() && self.done >= lines.end() => "no line".to_owned(),
            None => total,
        };
        Some(if self.done < lines.end() { format!("{text} so far") } else { text })
    }

    /// "Reading 40,000 of 200,000 lines" while the worker reads, or why it stopped.
    pub(crate) fn progress(&self, lines: &Lines) -> Option<String> {
        if self.needles == (None, None) {
            return None;
        }
        if let Some(failure) = self.worker.as_ref().and_then(Worker::failure) {
            return Some(format!("The search stopped: {failure}"));
        }
        let read = self.done.saturating_sub(lines.first()).min(lines.len() as u64);
        (self.done < lines.end()).then(|| {
            let total = crate::format::thousands(lines.len() as u64);
            format!("Reading {} of {total} lines", crate::format::thousands(read))
        })
    }
}

/// Removes the numbers of the lines that left the ring.
fn trim(numbers: &mut VecDeque<u64>, first: u64) {
    let gone = numbers.partition_point(|number| *number < first);
    numbers.drain(..gone);
}

/// Reads one part on the worker thread.
fn read(scan: &Scan, cancel: &Cancel<'_>) -> Option<Scanned> {
    let to = (scan.from + PART).min(scan.lines.end()).max(scan.from);
    let (mut kept, mut hits) = (Vec::new(), Vec::new());
    for (count, (number, line)) in scan.lines.range(scan.from, to).enumerate() {
        if count % STEP == 0 && cancel.is_set() {
            return None;
        }
        let marker = line.kind == LineKind::Marker;
        if !marker && scan.filter.as_ref().is_some_and(|filter| !filter.is_in(&line.text)) {
            continue;
        }
        if scan.filter.is_some() {
            kept.push(number);
        }
        if !marker && scan.search.as_ref().is_some_and(|search| search.is_in(&line.text)) {
            hits.push(number);
        }
    }
    Some(Scanned { from: scan.from, to, kept, hits })
}

#[cfg(test)]
mod tests;
