//! The view of one store that a reader holds. It does not change.

use std::time::SystemTime;

use super::items::Items;
use super::state::StoreState;
use crate::kinds::Kind;

/// All objects of one kind in one cluster at one moment, with the state of the store.
#[derive(Debug)]
pub struct Snapshot<T> {
    /// A number that grows each time the objects change, and only then. A view computes
    /// its rows again only for a new number.
    pub generation: u64,
    /// The state of the store.
    pub state: StoreState,
    /// The wall clock time of the last change of the state.
    pub since: SystemTime,
    /// The wall clock time of the last data of the server: a list page, an event or a
    /// bookmark. `None` before the first data. The UI tells from it how fresh the objects are.
    pub last_contact: Option<SystemTime>,
    /// True after the first complete list of the store. Before it, the store does not
    /// know the objects of the cluster: no object in it is "not loaded", not "none".
    pub listed: bool,
    /// The count of objects that left the store because of its limit. Only the store of
    /// the Events has a limit.
    pub dropped: u64,
    /// The objects.
    pub items: Items<T>,
}

impl<T> Snapshot<T> {
    /// A store with no object, before its first request.
    pub(crate) fn empty(state: StoreState, now: SystemTime) -> Self {
        Self {
            generation: 0,
            state,
            since: now,
            last_contact: None,
            listed: false,
            dropped: 0,
            items: Items::default(),
        }
    }

    /// The same objects in a new state.
    pub(crate) fn in_state(&self, state: StoreState, now: SystemTime) -> Self {
        Self {
            generation: self.generation,
            state,
            since: now,
            last_contact: self.last_contact,
            listed: self.listed,
            dropped: self.dropped,
            items: self.items.clone(),
        }
    }

    /// The facts of the store with no object.
    pub(super) fn status(&self, kind: Kind) -> StoreStatus {
        StoreStatus {
            kind,
            state: self.state.clone(),
            since: self.since,
            last_contact: self.last_contact,
            listed: self.listed,
            items: self.items.len(),
            dropped: self.dropped,
            generation: self.generation,
        }
    }
}

/// The facts of one store, with no object and no type of its objects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoreStatus {
    /// The kind of the store.
    pub kind: Kind,
    /// The state.
    pub state: StoreState,
    /// The wall clock time of the last change of the state.
    pub since: SystemTime,
    /// The wall clock time of the last data of the server.
    pub last_contact: Option<SystemTime>,
    /// True after the first complete list of the store.
    pub listed: bool,
    /// The count of objects.
    pub items: usize,
    /// The count of objects that left the store because of its limit.
    pub dropped: u64,
    /// The number of the last change of the objects.
    pub generation: u64,
}
