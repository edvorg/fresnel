//! One store as an index reads it, and what an answer of an index is good for.

use std::time::SystemTime;

use crate::kinds::{Kind, Object};
use crate::published::Reader;
use crate::store::{Change, Items, Snapshot, StoreState};

/// What an index knows of one store that it reads, at its last read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceStatus {
    /// The kind of the store.
    pub kind: Kind,
    /// True after the first complete list of the store. Before it, the index does not
    /// know the objects of the kind: an object that an answer does not have can exist.
    pub listed: bool,
    /// The state of the store. The objects of a store that is not live can be old.
    pub state: StoreState,
    /// The wall clock time of the last change of that state.
    pub since: SystemTime,
}

impl SourceStatus {
    /// What an index knows of a store after it read this snapshot of it.
    pub(super) fn of<T>(kind: Kind, snapshot: &Snapshot<T>) -> Self {
        Self { kind, listed: snapshot.listed, state: snapshot.state.clone(), since: snapshot.since }
    }
}

/// One store with the objects that the index read from it. The differences from these
/// objects to the next snapshot are the changes that the index must apply.
pub(super) struct Tracked<T> {
    kind: Kind,
    reader: Reader<Snapshot<T>>,
    seen: Items<T>,
}

impl<T: Object> Tracked<T> {
    /// A store that the index did not read yet: the first read gives each object as new.
    pub(super) fn new(kind: Kind, reader: Reader<Snapshot<T>>) -> Self {
        Self { kind, reader, seen: Items::default() }
    }

    /// Reads the snapshot of now and gives each change of the objects since the last read
    /// to `apply`. A difference of two snapshots cannot lose a change, and its cost
    /// follows the count of the changes.
    pub(super) fn read(&mut self, mut apply: impl FnMut(Change<'_, T>)) -> SourceStatus {
        let snapshot = self.reader.read();
        self.seen.changes_to(&snapshot.items).for_each(&mut apply);
        self.seen = snapshot.items.clone();
        SourceStatus::of(self.kind, &snapshot)
    }

    /// The objects of the last read.
    pub(super) fn seen(&self) -> &Items<T> {
        &self.seen
    }

    /// Waits for a new snapshot of the store: new objects or a new state.
    pub(super) async fn changed(&mut self) {
        self.reader.changed().await;
    }
}
