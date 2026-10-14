//! The stores of an index test: the test writes their objects, and an index reads them.

use std::sync::mpsc;
use std::time::{Duration, SystemTime};

use fresnel_net::WallClock;
use fresnel_net::task::{TaskReport, Tasks};
use tokio::runtime::Handle;

use super::task::{Index, Indexer};
use crate::fixture;
use crate::kinds::{Describe, Object};
use crate::published::{Bin, Disposal, Notice, Publisher, Reader};
use crate::store::{Snapshot, StoreState, Wait, Writer};

/// The time of each state of a test store.
pub(super) const EPOCH: SystemTime = SystemTime::UNIX_EPOCH;

/// The tasks of one test, with the thread that frees old values, and each report of a
/// task that ended.
pub(super) struct Bed {
    pub(super) tasks: Tasks,
    pub(super) reports: mpsc::Receiver<TaskReport>,
    disposal: Disposal,
}

impl Bed {
    pub(super) fn new() -> Self {
        let (sender, reports) = mpsc::channel();
        let tasks = Tasks::new(Handle::current(), move |report: TaskReport| sender.send(report).expect("a reader"));
        let disposal = Disposal::start(&tasks);
        Self { tasks, reports, disposal }
    }

    pub(super) fn bin(&self) -> Bin {
        self.disposal.bin()
    }

    /// A store with no object, before its first list.
    pub(super) fn store<T: Object>(&self) -> TestStore<T> {
        let first = Snapshot::empty(StoreState::Waiting(Wait::Channel), EPOCH);
        let publisher = Publisher::new(first, self.bin(), Notice::new());
        let reader = publisher.reader();
        let clock = WallClock::new(|| EPOCH);
        TestStore { writer: Writer::resume(publisher, clock, Duration::ZERO, None), reader }
    }

    /// A live store with each object of the fixture of its kind.
    pub(super) fn fixture<K: Describe>(&self) -> TestStore<K> {
        let mut store = self.store();
        store.list(fixture::all());
        store
    }
}

/// One store that a test writes. Each change shows to the readers at once.
pub(super) struct TestStore<T> {
    writer: Writer<T>,
    reader: Reader<Snapshot<T>>,
}

impl<T: Object> TestStore<T> {
    pub(super) fn reader(&self) -> Reader<Snapshot<T>> {
        self.reader.clone()
    }

    /// A complete list with these objects. Then the store is live.
    pub(super) fn list(&mut self, objects: Vec<T>) {
        self.writer.begin_list();
        self.writer.list_page(objects).expect("objects with names");
        self.writer.end_list();
        self.writer.set_state(StoreState::Live);
    }

    /// The watch event for a new object or a new version of one.
    pub(super) fn apply(&mut self, object: T) {
        self.writer.apply(object).expect("an object with a name");
        self.writer.flush();
    }

    /// The watch event for an object that is gone.
    pub(super) fn delete(&mut self, object: &T) {
        self.writer.delete(object).expect("an object with a name");
        self.writer.flush();
    }

    pub(super) fn set_state(&mut self, state: StoreState) {
        self.writer.set_state(state);
    }

    /// Data of the server with no change of an object, as a bookmark of a watch.
    pub(super) fn bookmark(&mut self) {
        self.writer.contact();
        self.writer.flush();
    }
}

/// Reads the stores of an indexer one time, and gives the index that its task would
/// publish. True when the read changed a relation.
pub(super) fn read<I: Indexer>(indexer: &mut I) -> (Index<I::Relations>, bool) {
    let read = indexer.read();
    (Index::following(u64::from(read.changed), read.sources, indexer.relations()), read.changed)
}
