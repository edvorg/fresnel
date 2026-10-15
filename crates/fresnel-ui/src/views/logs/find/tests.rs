//! The search and the filter on their worker thread, with views that grow and lose lines.

use std::sync::mpsc::{Receiver, channel};

use fresnel_core::logs::LogLine;

use super::*;
use crate::testing::{LIMIT, TestTasks};

fn line(text: &str, kind: LineKind) -> LogLine {
    LogLine { time: 0, source: 0, text: text.into(), spans: None, kind, late: false, cut: false }
}

/// `count` lines from the number `first`: each tenth one has "error", each line has its number.
fn lines(first: u64, count: u64) -> Lines {
    let text =
        |number: u64| if number.is_multiple_of(10) { format!("{number} ERROR here") } else { format!("{number} fine") };
    Lines::new(first, (first..first + count).map(|number| line(&text(number), LineKind::Log)))
}

struct Bed {
    find: Find,
    bed: TestTasks,
    wake: Wake,
    woken: Receiver<()>,
}

impl Bed {
    fn new() -> Self {
        let (sender, woken) = channel();
        let wake: Wake = Arc::new(move || if sender.send(()).is_err() {});
        Self { find: Find::default(), bed: TestTasks::new(), wake, woken }
    }

    /// Updates the find state with these lines until the worker read each one.
    fn read(&mut self, lines: &Lines) {
        self.find.update(lines, &self.bed.tasks, &self.wake);
        while self.find.done < lines.end() {
            self.woken.recv_timeout(LIMIT).expect("the worker gives a part");
            self.find.update(lines, &self.bed.tasks, &self.wake);
        }
    }

    fn hits(&self) -> Vec<u64> {
        self.find.hits.iter().copied().collect()
    }
}

#[test]
fn the_worker_reads_each_part_and_the_new_lines_and_a_new_text_starts_again() {
    let mut bed = Bed::new();
    bed.find.search = "error".to_owned();
    let first = lines(0, 45_000);
    bed.read(&first);
    assert_eq!(bed.hits().len(), 4_500, "three parts of the worker");
    assert_eq!(bed.find.kept(), None, "no filter");
    assert_eq!(bed.find.count().as_deref(), Some("4,500 hits"));

    // New lines and lines that left the ring.
    let later = lines(5_000, 41_000);
    bed.read(&later);
    assert_eq!((bed.hits().first(), bed.hits().last(), bed.hits().len()), (Some(&5_000), Some(&45_990), 4_100));

    bed.find.search = "Error".to_owned();
    bed.read(&later);
    assert_eq!(bed.hits(), [] as [u64; 0], "a capital letter keeps the case");
    assert_eq!(bed.find.count().as_deref(), Some("no hit"));
}

#[test]
fn the_filter_keeps_its_lines_and_each_marker_and_the_search_finds_hits_among_them() {
    let mut bed = Bed::new();
    let mut all: Vec<LogLine> = (0..6).map(|number| line(&format!("{number} fine"), LineKind::Log)).collect();
    all.push(line("the stream started again", LineKind::Marker));
    all.extend(["GET /a", "POST /b", "GET /c"].map(|text| line(text, LineKind::Log)));
    let view = Lines::new(100, all);
    (bed.find.filter, bed.find.search) = ("/".to_owned(), "get".to_owned());
    bed.read(&view);
    let kept: Vec<u64> = bed.find.kept().map(|kept| kept.iter().copied().collect()).unwrap_or_default();
    assert_eq!(kept, [106, 107, 108, 109]);
    assert_eq!(bed.hits(), [107, 109]);

    assert_eq!(bed.find.go(true, 0), Some(107));
    assert_eq!(bed.find.go(true, 0), Some(109));
    assert_eq!(bed.find.go(true, 0), Some(107), "after the last hit comes the first one");
    assert_eq!(bed.find.go(false, 0), Some(109));
    assert_eq!(bed.find.count().as_deref(), Some("2 of 2 hits"));
    bed.find.current = None;
    assert_eq!(bed.find.go(true, 108), Some(109), "no current hit: from the line of the pane");
}

/// The pane shows the kept lines: a new search that emptied them would move the pane to the
/// end of the first part that the worker gives again.
#[test]
fn a_new_search_keeps_the_lines_of_the_same_filter_and_a_new_filter_reads_them_again() {
    let mut bed = Bed::new();
    bed.find.filter = "error".to_owned();
    let view = lines(0, 45_000);
    bed.read(&view);
    let kept = |find: &Find| find.kept().map(|kept| kept.iter().copied().collect::<Vec<u64>>()).unwrap_or_default();
    let before = kept(&bed.find);
    assert_eq!(before.len(), 4_500);

    bed.find.search = "4".to_owned();
    bed.find.update(&view, &bed.bed.tasks, &bed.wake);
    assert_eq!(kept(&bed.find), before, "the kept lines stay while the worker reads again");
    bed.read(&view);
    assert_eq!(kept(&bed.find), before, "no line two times");
    let with_4 = (0..45_000_u64).step_by(10).filter(|number| number.to_string().contains('4')).count();
    assert_eq!(bed.hits().len(), with_4);

    bed.find.filter = "fine".to_owned();
    bed.find.update(&view, &bed.bed.tasks, &bed.wake);
    assert_eq!(kept(&bed.find), [] as [u64; 0], "a new filter keeps other lines");
    bed.read(&view);
    assert_eq!(kept(&bed.find).len(), 40_500);
}

#[test]
fn a_view_that_starts_again_with_fewer_lines_gets_a_new_search() {
    let mut bed = Bed::new();
    bed.find.search = "error".to_owned();
    bed.read(&lines(0, 1_000));
    assert_eq!(bed.hits().len(), 100);
    // The task of the view started again: its lines have new numbers from 0.
    bed.read(&lines(0, 30));
    assert_eq!(bed.hits(), [0, 10, 20]);
    assert_eq!(bed.find.progress(&lines(0, 30)), None, "each line is read");
    assert!(bed.bed.reports.try_recv().is_err(), "the worker did not end");
}
