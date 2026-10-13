//! The task of one index, and the value of the index that a reader holds.
//!
//! The task reads its stores, publishes the index and waits for the next snapshot of a
//! store. A panic of the task leaves the last index with the state "failed" (design rule
//! 1). The start function then gives the report for the journal and starts the task
//! again, and the new run builds the relations from the full snapshots.

use std::convert::Infallible;
use std::panic::{AssertUnwindSafe, resume_unwind};
use std::sync::Arc;

use fresnel_net::task::panic_message;
use futures_util::FutureExt as _;

use super::source::SourceStatus;
use crate::kinds::Kind;
use crate::published::Publisher;

/// What the task of an index does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IndexState {
    /// The task did not read its stores yet.
    Starting,
    /// The task follows its stores: the index has each change of their snapshots.
    Following,
    /// The task failed, for example with a panic. It starts again. Until then, the
    /// index is the one from before the fault.
    Failed {
        /// What failed.
        reason: Arc<str>,
    },
    /// The app stopped the index. It does not change again.
    Stopped,
}

/// What an answer of an index is good for (design rules 4 and 5). A view shows this next
/// to the answer, so that "none" and "not known" never look the same.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Basis {
    /// Each store behind the answer is live: the answer is the state of the cluster.
    Current,
    /// Each store behind the answer had a complete list, and this one is not live now.
    /// The answer can be old.
    Old(SourceStatus),
    /// This store had no complete list yet. The answer can miss objects: an empty
    /// answer means "not loaded", not "none".
    NotLoaded(SourceStatus),
    /// The task of the index does not follow its stores. The answer is from before.
    NotFollowed(IndexState),
}

impl Basis {
    /// True when the answer had each object of the cluster at some moment: an empty
    /// answer then means "none".
    pub fn is_loaded(&self) -> bool {
        matches!(self, Self::Current | Self::Old(_))
    }
}

/// One answer of an index, with what it is good for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer<T> {
    /// What the index found.
    pub found: T,
    /// What the answer is good for.
    pub basis: Basis,
}

/// One index of a cluster as a reader holds it. It does not change.
#[derive(Clone, Debug)]
pub struct Index<R> {
    /// A number that grows each time a relation changes, and with the first relations of a
    /// task that started again. A view computes again what it read from the index only
    /// for a new number.
    pub generation: u64,
    /// What the task of the index does.
    pub state: IndexState,
    sources: Arc<[SourceStatus]>,
    relations: R,
}

impl<R> Index<R> {
    /// The index before the first read of its stores.
    pub(super) fn starting(relations: R) -> Self {
        Self { generation: 0, state: IndexState::Starting, sources: Arc::new([]), relations }
    }

    /// The index after a read of its stores.
    pub(super) fn following(generation: u64, sources: Vec<SourceStatus>, relations: R) -> Self {
        Self { generation, state: IndexState::Following, sources: sources.into(), relations }
    }

    /// What the index knows of each store that it reads.
    pub fn sources(&self) -> &[SourceStatus] {
        &self.sources
    }

    pub(super) fn relations(&self) -> &R {
        &self.relations
    }

    /// What an answer is good for that comes from the stores of these kinds. The first
    /// store with no complete list decides, then the first one that is not live.
    pub(super) fn basis(&self, kinds: &[Kind]) -> Basis {
        if self.state != IndexState::Following {
            return Basis::NotFollowed(self.state.clone());
        }
        let behind = || self.sources.iter().filter(|source| kinds.contains(&source.kind));
        let not_loaded = behind().find(|source| !source.listed).cloned().map(Basis::NotLoaded);
        let old = || behind().find(|source| !source.state.is_live()).cloned().map(Basis::Old);
        not_loaded.or_else(old).unwrap_or(Basis::Current)
    }

    /// The same index in a new state of its task.
    pub(super) fn in_state(&self, state: IndexState) -> Self
    where
        R: Clone,
    {
        Self {
            generation: self.generation,
            state,
            sources: Arc::clone(&self.sources),
            relations: self.relations.clone(),
        }
    }
}

/// What one read of the stores of an index gave.
pub(super) struct Read {
    /// True when a relation of the index changed.
    pub(super) changed: bool,
    /// What the index knows of each store now, in a fixed sequence.
    pub(super) sources: Vec<SourceStatus>,
}

/// The write side of one index: it reads its stores and holds the relations.
pub(super) trait Indexer: Send + 'static {
    /// The relations, as a reader gets them. A copy shares the data.
    type Relations: Clone + Default + Send + Sync + 'static;

    /// Reads each store now and applies the changes of its objects to the relations.
    fn read(&mut self) -> Read;

    /// The relations after the last read.
    fn relations(&self) -> Self::Relations;

    /// Waits until a store has a new snapshot.
    fn changed(&mut self) -> impl Future<Output = ()> + Send;
}

/// One run of the task of an index. `indexer` starts with no relation, so the first read
/// builds each one again. A run ends only with a panic, which goes on to the start
/// function after the index shows the fault.
pub(super) async fn run<I: Indexer>(indexer: I, publisher: Publisher<Index<I::Relations>>) -> Result<(), Infallible> {
    let panic = match AssertUnwindSafe(follow(indexer, &publisher)).catch_unwind().await {
        Ok(never) => match never {},
        Err(panic) => panic,
    };
    let reason = format!("the task of the index panicked: {}", panic_message(panic.as_ref()));
    let failed = publisher.read().in_state(IndexState::Failed { reason: reason.into() });
    publisher.publish(failed);
    resume_unwind(panic)
}

/// Follows the stores with no end. The readers get a new index when a relation changed
/// or when the index knows a new fact of a store.
async fn follow<I: Indexer>(mut indexer: I, publisher: &Publisher<Index<I::Relations>>) -> Infallible {
    // A run after a fault continues the count, and its first relations get a new number
    // also when the read found no change: the run built them from nothing, so they can
    // differ from the ones that a view read before. Without it, the objects that left
    // the stores during the fault would stay in that view.
    let (mut generation, mut rebuilt) = {
        let last = publisher.read();
        (last.generation, last.state != IndexState::Starting)
    };
    let mut published: Option<Vec<SourceStatus>> = None;
    loop {
        let read = indexer.read();
        let new_relations = read.changed || rebuilt;
        rebuilt = false;
        if new_relations || published.as_ref() != Some(&read.sources) {
            generation += u64::from(new_relations);
            publisher.publish(Index::following(generation, read.sources.clone(), indexer.relations()));
            published = Some(read.sources);
        }
        indexer.changed().await;
    }
}

#[cfg(test)]
mod tests;
