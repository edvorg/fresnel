//! What the worker of the quick search reads, and when the stores changed.
//!
//! A request holds a snapshot of each store of each selected cluster. A new snapshot of a
//! store, a new state and a cluster that comes or goes give a new request, so a hit gets
//! its mark in the frame after its store left the state "live" (design rule 4).

use std::hash::{DefaultHasher, Hash, Hasher};

use fresnel_core::app::CoreView;
use fresnel_core::search::{ClusterSources, Query};

/// One request to the worker.
#[derive(Clone)]
pub(super) struct Request {
    pub(super) query: Query,
    pub(super) clusters: Vec<ClusterSources>,
}

impl Request {
    /// The query and the stores of each selected cluster. The call copies one pointer for
    /// each store.
    pub(super) fn new(query: &Query, view: &CoreView) -> Self {
        let clusters = view.clusters.iter().filter(|row| row.selected).map(|row| row.search_sources()).collect();
        Self { query: query.clone(), clusters }
    }
}

/// The mark of the stores of the last frame, and a number that grows when it changes.
#[derive(Debug, Default)]
pub(super) struct Stores {
    mark: Option<u64>,
    serial: u64,
}

impl Stores {
    /// The number of the stores of this frame.
    pub(super) fn serial(&mut self, view: &CoreView) -> u64 {
        let now = mark(view);
        if self.mark != Some(now) {
            self.mark = Some(now);
            self.serial += 1;
        }
        self.serial
    }
}

/// A mark of what the search reads of the selected clusters: the identity of each store,
/// the number of its objects and the facts that the notes and the marks show.
fn mark(view: &CoreView) -> u64 {
    let mut hasher = DefaultHasher::new();
    for row in view.clusters.iter().filter(|row| row.selected) {
        row.id.hash(&mut hasher);
        let Some(data) = &row.data else {
            row.reason().hash(&mut hasher);
            continue;
        };
        let shown = data.objects.kinds().filter_map(|kind| data.objects.status(kind));
        for status in data.stores.iter().chain(shown) {
            (status.kind, status.generation, status.listed, status.last_contact).hash(&mut hasher);
            status.state.to_string().hash(&mut hasher);
        }
    }
    hasher.finish()
}
