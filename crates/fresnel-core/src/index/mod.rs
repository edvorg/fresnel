//! The indexes of one cluster: the relations between its objects that a view asks for
//! (plan sections 9.6, 9.8 and 9.9).
//!
//! An index is derived data. One task for each index reads the snapshots of its stores and
//! applies the differences between two snapshots. A reader gets an [`Index`] through the
//! published state type. Each answer tells what it is good for ([`Basis`]): "no pods" and
//! "the pods are not loaded" are two answers.
//!
//! - `owner`: from an owner to its objects, and from a Service to its pods.
//! - `refs`: from a ConfigMap, a Secret, a ServiceAccount or a Node to its pods.
//! - `events`: from an object to its Events.
//! - `task`: the task of an index, and what a reader holds. `source`: one store as an
//!   index reads it. `links`: the map of one relation.

#[cfg(test)]
mod bed;
mod events;
mod links;
mod owner;
mod refs;
mod source;
mod task;

use fresnel_net::task::{Backoff, Restart, TaskHandle, TaskName, Tasks};

pub use events::ObjectEvents;
pub use owner::{EndpointPod, ObjectRef, Owners};
pub use refs::{References, Target, references_of};
pub use source::SourceStatus;
pub use task::{Answer, Basis, Index, IndexState};

use crate::cluster::ClusterId;
use crate::error::CoreError;
use crate::kinds::Full;
use crate::published::{Bin, Changes, Notice, Publisher, Reader};
use crate::store::{ClusterStores, Snapshot};
use events::EventIndexer;
use owner::OwnerIndexer;
use refs::ReferenceIndexer;
use task::Indexer;

/// What the owner of the indexes gives to them.
#[derive(Clone)]
pub struct IndexParts {
    /// Frees the values of an index that the readers held.
    pub bin: Bin,
    /// The pauses before the task of an index starts again after a fault of its own.
    pub restart: Backoff,
}

/// One index with its task. The task stops when this value drops, and the readers of the
/// index then see the state "stopped" with the last relations.
struct Running<R: Clone + Send + Sync + 'static> {
    publisher: Publisher<Index<R>>,
    _task: TaskHandle,
}

impl<R: Clone + Send + Sync + 'static> Drop for Running<R> {
    fn drop(&mut self) {
        self.publisher.close(|last| last.in_state(IndexState::Stopped));
    }
}

/// Starts the indexes of one cluster.
struct Starter<'a> {
    tasks: &'a Tasks,
    cluster: &'a ClusterId,
    parts: IndexParts,
    notice: Notice,
}

impl Starter<'_> {
    /// Starts one index. `indexer` builds the write side of one run of the task, with no
    /// relation: a run after a fault reads each store from its start.
    fn start<I: Indexer>(&self, role: &'static str, indexer: impl Fn() -> I + Send + 'static) -> Running<I::Relations> {
        let first = Index::starting(I::Relations::default());
        let publisher = Publisher::new(first, self.parts.bin.clone(), self.notice.clone());
        let name = TaskName::scoped(role, self.cluster.as_str());
        let task = self.tasks.start(name, Restart::Always(self.parts.restart), {
            let publisher = publisher.clone();
            move || task::run(indexer(), publisher.clone())
        });
        Running { publisher, _task: task }
    }
}

/// The store of a tier A kind that an index reads.
fn store<K: Full>(stores: &ClusterStores) -> Result<Reader<Snapshot<K>>, CoreError> {
    let absent = || CoreError::invalid(format!("the cluster has no store of the kind {}", K::KIND.info().name));
    stores.full::<K>().ok_or_else(absent)
}

/// The indexes of one cluster. Each one stops when this value drops.
pub struct ClusterIndexes {
    owners: Running<Owners>,
    references: Running<References>,
    events: Option<Running<ObjectEvents>>,
    notice: Notice,
}

impl ClusterIndexes {
    /// Starts the indexes on the stores of a cluster, with one task for each index.
    pub fn start(tasks: &Tasks, stores: &ClusterStores, parts: IndexParts) -> Result<Self, CoreError> {
        let starter = Starter { tasks, cluster: stores.cluster(), parts, notice: Notice::new() };
        let (replica_sets, jobs, pods, slices) = (store(stores)?, store(stores)?, store(stores)?, store(stores)?);
        let owners = starter.start("owner index", {
            let pods = pods.clone();
            move || OwnerIndexer::new(replica_sets.clone(), jobs.clone(), pods.clone(), slices.clone())
        });
        let references = starter.start("reference index", move || ReferenceIndexer::new(pods.clone()));
        let events =
            stores.full().map(|events| starter.start("event index", move || EventIndexer::new(events.clone())));
        Ok(Self { owners, references, events, notice: starter.notice })
    }

    /// Tells when an index of the cluster has a new value.
    pub fn changes(&self) -> Changes {
        self.notice.changes()
    }

    /// A reader of the owner index.
    pub fn owners(&self) -> Reader<Index<Owners>> {
        self.owners.publisher.reader()
    }

    /// A reader of the reference index.
    pub fn references(&self) -> Reader<Index<References>> {
        self.references.publisher.reader()
    }

    /// A reader of the Event index. `None` when the settings give the Events no store:
    /// a view then reads the Events of its object from the server.
    pub fn events(&self) -> Option<Reader<Index<ObjectEvents>>> {
        self.events.as_ref().map(|events| events.publisher.reader())
    }
}

impl std::fmt::Debug for ClusterIndexes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClusterIndexes").field("events", &self.events.is_some()).finish_non_exhaustive()
    }
}
