//! The journal: the list of events, warnings and errors that the app shows.
//!
//! The list in memory has a limit. An entry that leaves the list stays in the log file, and
//! the view tells how many entries left. A text of an entry is safe for a log: it never
//! holds a token, the output of a plugin or a value of an object.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::SystemTime;

use fresnel_net::WallClock;

/// The weight of one entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    /// Something that the app did.
    Info,
    /// A fault that the app handles by itself.
    Warning,
    /// A fault that needs the user.
    Error,
}

/// One entry of the journal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The number of the entry from the start of the app, from 0.
    pub number: u64,
    /// The wall clock time of the entry.
    pub time: SystemTime,
    /// The weight.
    pub level: Level,
    /// Who reports: the app, a cluster or a task.
    pub source: String,
    /// What occurred, on one line or more.
    pub text: String,
}

/// Takes each entry of the journal, for example the log file. `write` runs in the task
/// that makes the entry, so it must return fast.
pub trait Sink: Send + 'static {
    /// Stores one entry. The entries arrive in the sequence of their numbers.
    fn write(&mut self, entry: &Entry);

    /// Makes each stored entry safe against a loss of power. The app calls it at its quit.
    fn flush(&mut self) {}
}

/// A copy of the journal for the UI.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JournalView {
    /// The entries in memory, the oldest first.
    pub entries: Vec<Arc<Entry>>,
    /// The count of all entries from the start of the app.
    pub total: u64,
    /// The count of all warnings from the start of the app.
    pub warnings: u64,
    /// The count of all errors from the start of the app.
    pub errors: u64,
}

impl JournalView {
    /// The count of entries that left the memory. Only the log file has them.
    pub fn dropped(&self) -> u64 {
        self.total.saturating_sub(self.entries.len() as u64)
    }
}

struct Ring {
    entries: VecDeque<Arc<Entry>>,
    total: u64,
    warnings: u64,
    errors: u64,
    /// The copy for the UI. A new entry removes it, so a frame with no new entry copies nothing.
    view: Option<Arc<JournalView>>,
}

struct Shared {
    limit: usize,
    clock: WallClock,
    ring: Mutex<Ring>,
    /// Its lock also puts the entries of two tasks into one sequence. It is not the lock
    /// of the list, so a reader of the list never waits for a write to the disk.
    sink: Mutex<Option<Box<dyn Sink>>>,
}

/// The journal. A clone is one more handle to the same list.
#[derive(Clone)]
pub struct Journal {
    shared: Arc<Shared>,
}

impl Journal {
    /// A journal that keeps the last `limit` entries in memory, and one entry at least.
    pub fn new(limit: usize, clock: WallClock) -> Self {
        let ring = Ring { entries: VecDeque::new(), total: 0, warnings: 0, errors: 0, view: None };
        let shared = Shared { limit: limit.max(1), clock, ring: Mutex::new(ring), sink: Mutex::new(None) };
        Self { shared: Arc::new(shared) }
    }

    /// The same, with a sink that takes each entry.
    pub fn with_sink(limit: usize, clock: WallClock, sink: impl Sink) -> Self {
        let journal = Self::new(limit, clock);
        *lock(&journal.shared.sink) = Some(Box::new(sink));
        journal
    }

    /// Adds one entry.
    pub fn push(&self, level: Level, source: impl Into<String>, text: impl Into<String>) {
        let mut sink = lock(&self.shared.sink);
        let entry = self.store(level, source.into(), text.into());
        if let Some(sink) = sink.as_mut() {
            sink.write(&entry);
        }
    }

    /// Adds an entry with the level `Info`.
    pub fn info(&self, source: impl Into<String>, text: impl Into<String>) {
        self.push(Level::Info, source, text);
    }

    /// Adds an entry with the level `Warning`.
    pub fn warning(&self, source: impl Into<String>, text: impl Into<String>) {
        self.push(Level::Warning, source, text);
    }

    /// Adds an entry with the level `Error`.
    pub fn error(&self, source: impl Into<String>, text: impl Into<String>) {
        self.push(Level::Error, source, text);
    }

    /// Tells the sink to make its entries safe. The quit sequence of the app calls it.
    pub fn flush(&self) {
        if let Some(sink) = lock(&self.shared.sink).as_mut() {
            sink.flush();
        }
    }

    /// A copy of the list and its counts. Two calls with no entry between them give the
    /// same copy.
    pub fn view(&self) -> Arc<JournalView> {
        let mut ring = lock(&self.shared.ring);
        if let Some(view) = &ring.view {
            return Arc::clone(view);
        }
        let view = Arc::new(JournalView {
            entries: ring.entries.iter().cloned().collect(),
            total: ring.total,
            warnings: ring.warnings,
            errors: ring.errors,
        });
        ring.view = Some(Arc::clone(&view));
        view
    }

    fn store(&self, level: Level, source: String, text: String) -> Arc<Entry> {
        let time = self.shared.clock.now();
        let mut ring = lock(&self.shared.ring);
        let entry = Arc::new(Entry { number: ring.total, time, level, source, text });
        ring.total += 1;
        match level {
            Level::Info => {}
            Level::Warning => ring.warnings += 1,
            Level::Error => ring.errors += 1,
        }
        if ring.entries.len() >= self.shared.limit {
            ring.entries.pop_front();
        }
        ring.entries.push_back(Arc::clone(&entry));
        ring.view = None;
        entry
    }
}

impl std::fmt::Debug for Journal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Journal").field("limit", &self.shared.limit).finish_non_exhaustive()
    }
}

/// Each change under these locks is complete or not started, so a poisoned lock is safe to use.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn clock() -> WallClock {
        WallClock::new(|| SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000))
    }

    /// Keeps the text of each entry that it gets.
    struct Lines(Arc<Mutex<Vec<String>>>);

    impl Sink for Lines {
        fn write(&mut self, entry: &Entry) {
            lock(&self.0).push(format!("{} {}", entry.number, entry.text));
        }

        fn flush(&mut self) {
            lock(&self.0).push("flush".to_owned());
        }
    }

    #[test]
    fn the_list_has_a_limit_and_the_counts_and_the_sink_keep_each_entry() {
        let written = Arc::new(Mutex::new(Vec::new()));
        let journal = Journal::with_sink(3, clock(), Lines(Arc::clone(&written)));
        journal.info("app", "one");
        journal.warning("ams", "two");
        journal.error("ams", "three");
        let full = journal.view();
        assert_eq!((full.total, full.warnings, full.errors, full.dropped()), (3, 1, 1, 0));

        journal.warning("fra", "four");
        journal.error("fra", "five");
        let view = journal.view();
        let texts: Vec<(u64, Level, &str, &str)> = view
            .entries
            .iter()
            .map(|entry| (entry.number, entry.level, entry.source.as_str(), entry.text.as_str()))
            .collect();
        assert_eq!(
            texts,
            [(2, Level::Error, "ams", "three"), (3, Level::Warning, "fra", "four"), (4, Level::Error, "fra", "five")]
        );
        assert_eq!((view.total, view.warnings, view.errors, view.dropped()), (5, 2, 2, 2));
        assert_eq!(view.entries[0].time, clock().now());
        journal.flush();
        assert_eq!(*lock(&written), ["0 one", "1 two", "2 three", "3 four", "4 five", "flush"]);
        Journal::new(3, clock()).flush();
        assert_eq!(full.entries.len(), 3, "a copy does not change after it was made");
    }

    #[test]
    fn a_view_with_no_new_entry_is_the_same_copy() {
        let journal = Journal::new(0, clock());
        assert_eq!(*journal.view(), JournalView::default());
        journal.info("app", "one");
        let (first, second) = (journal.view(), journal.view());
        assert!(Arc::ptr_eq(&first, &second));
        journal.info("app", "two");
        let third = journal.view();
        assert!(!Arc::ptr_eq(&first, &third));
        assert_eq!((third.entries.len(), third.total, third.dropped()), (1, 2, 1), "a limit of 0 keeps one entry");
    }

    #[test]
    fn entries_of_many_threads_reach_the_sink_in_the_sequence_of_their_numbers() {
        let written = Arc::new(Mutex::new(Vec::new()));
        let journal = Journal::with_sink(10, clock(), Lines(Arc::clone(&written)));
        std::thread::scope(|scope| {
            for thread in 0..8 {
                let journal = journal.clone();
                scope.spawn(move || (0..200).for_each(|_| journal.info("app", thread.to_string())));
            }
        });
        let numbers: Vec<u64> = lock(&written)
            .iter()
            .map(|line| line.split(' ').next().and_then(|number| number.parse().ok()).expect("a number"))
            .collect();
        assert_eq!(numbers, (0..1600).collect::<Vec<u64>>());
        assert_eq!(journal.view().dropped(), 1590);
    }
}
