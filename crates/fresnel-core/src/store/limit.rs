//! The objects of one store, with a limit for their count (plan section 9.6).
//!
//! Events can be most of the objects of a cluster: 32,000 of 47,000 in the cluster of the
//! store prototype. The store of the Events keeps the newest ones to a maximum count, and
//! its snapshot shows how many left (design rule 12).

use std::collections::BTreeSet;
use std::sync::Arc;

use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;

use super::items::{Items, ObjectKey};
use crate::kinds::Object;

/// What an insert did.
pub(super) enum Insert {
    /// The object is in the store. `left` is the count of older objects that left for the limit.
    Done { left: usize },
    /// The object is older than each object of a full store. It did not go in.
    TooOld,
}

impl Insert {
    /// The count of objects that the store does not have because of its limit.
    pub(super) fn dropped(&self) -> usize {
        match self {
            Self::Done { left } => *left,
            Self::TooOld => 1,
        }
    }
}

pub(super) struct Limited<T> {
    items: Items<T>,
    /// The most objects. `None`: no limit, and `by_age` stays empty.
    max: Option<usize>,
    /// The creation time of each object, so that the oldest one leaves first.
    by_age: BTreeSet<(Option<Time>, ObjectKey)>,
}

impl<T: Object> Limited<T> {
    pub(super) fn new(max: Option<usize>) -> Self {
        Self { items: Items::default(), max, by_age: BTreeSet::new() }
    }

    /// The objects of a snapshot, which had the same limit.
    pub(super) fn of(items: Items<T>, max: Option<usize>) -> Self {
        let ages = items.iter().filter(|_| max.is_some()).map(|(key, object)| age(key, object));
        Self { by_age: ages.collect(), items, max }
    }

    pub(super) fn items(&self) -> &Items<T> {
        &self.items
    }

    /// Adds or replaces one object. In a full store, the oldest objects leave.
    pub(super) fn insert(&mut self, key: ObjectKey, object: Arc<T>) -> Insert {
        let Some(max) = self.max else {
            self.items.insert(key, object);
            return Insert::Done { left: 0 };
        };
        let new = age(&key, &object);
        match self.items.get(&key).map(|known| age(&key, known)) {
            Some(known) => drop(self.by_age.remove(&known)),
            None if self.items.len() >= max && self.by_age.first().is_some_and(|oldest| new < *oldest) => {
                return Insert::TooOld;
            }
            None => {}
        }
        self.by_age.insert(new);
        self.items.insert(key, object);
        let mut left = 0;
        while self.items.len() > max
            && let Some((_, oldest)) = self.by_age.pop_first()
        {
            self.items.remove(&oldest);
            left += 1;
        }
        Insert::Done { left }
    }

    /// Removes one object. True when the store had it.
    pub(super) fn remove(&mut self, key: &ObjectKey) -> bool {
        if let Some(known) = self.items.get(key).filter(|_| self.max.is_some()) {
            let known = age(key, known);
            self.by_age.remove(&known);
        }
        self.items.remove(key)
    }
}

/// The place of an object in the sequence by age. The identity makes each entry unique.
fn age<T: Object>(key: &ObjectKey, object: &Arc<T>) -> (Option<Time>, ObjectKey) {
    (object.meta().creation_timestamp.clone(), key.clone())
}
