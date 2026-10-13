//! Stores: the copy in memory of all objects of one kind in one cluster (plan section 9).
//!
//! One task for each store runs the watch loop and owns the objects. A reader gets a
//! [`Snapshot`] through the published state type, so it never waits for the task.
//!
//! - `cluster`: the stores of one cluster. `writer`: the write side of one store.
//! - `snapshot`, `items`, `state`: what a reader sees. `strip` and `limit`: what a store
//!   does not keep. `pending`: the result of a write that a snapshot does not show yet.

mod cluster;
mod items;
mod limit;
mod pending;
mod snapshot;
mod state;
mod strip;
mod writer;

pub use cluster::{ClusterStores, EventsMode, StoreParts, StoreSettings};
pub use items::{Items, ObjectKey};
pub use pending::Written;
pub use snapshot::{Snapshot, StoreStatus};
pub use state::{StoreState, Wait};
pub(crate) use writer::Writer;

pub use crate::watch::WatchSettings;
