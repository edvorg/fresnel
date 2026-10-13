//! The write side of one store. The task of the store owns it, so the objects need no lock.
//!
//! The writer applies events at once and publishes them in batches (plan section 9.2): at
//! 200 pod changes in one second, the readers get 30 snapshots and not 200.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use fresnel_net::WallClock;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use tokio::time::Instant;

use super::items::ObjectKey;
use super::limit::{Insert, Limited};
use super::snapshot::Snapshot;
use super::state::StoreState;
use super::strip::strip;
use crate::error::CoreError;
use crate::kinds::Object;
use crate::published::Publisher;

/// The objects of a list in progress. The readers see the objects before the list until
/// the list is complete (design rule 5).
struct Listing<T> {
    items: Limited<T>,
    /// True when an object of the list is new or has a new version.
    changed: bool,
    /// The objects of the list that left for the limit of the store.
    dropped: usize,
}

pub(crate) struct Writer<T> {
    publisher: Publisher<Snapshot<T>>,
    clock: WallClock,
    /// The shortest time between two snapshots with new objects.
    batch: Duration,
    /// The most objects of the store. `None`: no limit.
    max_items: Option<usize>,
    items: Limited<T>,
    listing: Option<Listing<T>>,
    state: StoreState,
    since: SystemTime,
    generation: u64,
    dropped: u64,
    last_contact: Option<SystemTime>,
    /// True when the objects differ from the objects of the last snapshot.
    changed: bool,
    /// True when the last snapshot does not have the last data of the server.
    unpublished: bool,
    last_publish: Option<Instant>,
}

impl<T: Object> Writer<T> {
    /// A writer that continues from the last snapshot of the store. A task that starts
    /// again keeps the objects that the readers see. `max_items` is the limit for the count
    /// of objects.
    pub(crate) fn resume(
        publisher: Publisher<Snapshot<T>>,
        clock: WallClock,
        batch: Duration,
        max_items: Option<usize>,
    ) -> Self {
        let last = publisher.read();
        Self {
            items: Limited::of(last.items.clone(), max_items),
            listing: None,
            state: last.state.clone(),
            since: last.since,
            generation: last.generation,
            dropped: last.dropped,
            last_contact: last.last_contact,
            changed: false,
            unpublished: false,
            last_publish: None,
            publisher,
            clock,
            batch,
            max_items,
        }
    }

    pub(crate) fn state(&self) -> &StoreState {
        &self.state
    }

    /// Changes the state and shows it to the readers at once, with each change that waits
    /// for its batch. A watch that ends changes the state, so a change never waits for the
    /// next request or for a pause.
    pub(crate) fn set_state(&mut self, state: StoreState) {
        if self.state != state {
            self.state = state;
            self.since = self.clock.now();
            self.publish();
        }
    }

    /// Starts a list. A list that was in progress is gone.
    pub(crate) fn begin_list(&mut self) {
        self.listing = Some(self.new_listing());
    }

    fn new_listing(&self) -> Listing<T> {
        Listing { items: Limited::new(self.max_items), changed: false, dropped: 0 }
    }

    /// Takes one page of the list. Gives the count of objects of the list so far.
    pub(crate) fn list_page(&mut self, objects: Vec<T>) -> Result<usize, CoreError> {
        let mut listing = self.listing.take().unwrap_or_else(|| self.new_listing());
        for mut object in objects {
            let key = prepare(&mut object)?;
            // An object with no change keeps its allocation, so the drop of the snapshot
            // before the list frees few objects.
            let known = self.items.items().get(&key).filter(|known| same_version(known.meta(), object.meta()));
            let entry = known.cloned().unwrap_or_else(|| {
                listing.changed = true;
                Arc::new(object)
            });
            listing.dropped += listing.items.insert(key, entry).dropped();
        }
        let loaded = listing.items.items().len();
        self.listing = Some(listing);
        self.contact();
        Ok(loaded)
    }

    /// Replaces the objects with the complete list in one step. A list with no change
    /// keeps the generation, so no view computes its rows again.
    pub(crate) fn end_list(&mut self) {
        if let Some(listing) = self.listing.take() {
            if listing.changed || listing.items.items().len() != self.items.items().len() {
                self.items = listing.items;
                self.changed = true;
            }
            self.count_dropped(listing.dropped);
            self.publish();
        }
    }

    fn count_dropped(&mut self, dropped: usize) {
        self.dropped = self.dropped.saturating_add(u64::try_from(dropped).unwrap_or(u64::MAX));
    }

    /// Drops a list that failed. The objects before the list stay.
    pub(crate) fn cancel_list(&mut self) {
        self.listing = None;
    }

    /// Adds or replaces the object of one event.
    pub(crate) fn apply(&mut self, mut object: T) -> Result<(), CoreError> {
        let key = prepare(&mut object)?;
        if !self.items.items().get(&key).is_some_and(|known| same_version(known.meta(), object.meta())) {
            let insert = self.items.insert(key, Arc::new(object));
            // An object that is too old for a full store changes nothing.
            self.changed |= matches!(insert, Insert::Done { .. });
            self.count_dropped(insert.dropped());
        }
        self.contact();
        Ok(())
    }

    /// Removes the object of one event.
    pub(crate) fn delete(&mut self, object: &T) -> Result<(), CoreError> {
        let key = ObjectKey::of(object.meta()).ok_or_else(no_name)?;
        self.changed |= self.items.remove(&key);
        self.contact();
        Ok(())
    }

    /// Records data of the server: an event, a bookmark or a list page.
    pub(crate) fn contact(&mut self) {
        self.last_contact = Some(self.clock.now());
        self.unpublished = true;
    }

    /// The time at which the readers must get what the last snapshot does not have. `None`
    /// when they have all. The first change after a quiet time is due at once.
    pub(crate) fn due(&self) -> Option<Instant> {
        self.unpublished.then(|| self.last_publish.map_or_else(Instant::now, |last| last + self.batch))
    }

    /// Publishes what waits, if the batch time is over.
    pub(crate) fn publish_if_due(&mut self) {
        if self.due().is_some_and(|due| due <= Instant::now()) {
            self.publish();
        }
    }

    fn publish(&mut self) {
        self.generation += u64::from(std::mem::take(&mut self.changed));
        self.unpublished = false;
        self.last_publish = Some(Instant::now());
        self.publisher.publish(Snapshot {
            generation: self.generation,
            state: self.state.clone(),
            since: self.since,
            last_contact: self.last_contact,
            dropped: self.dropped,
            items: self.items.items().clone(),
        });
    }
}

/// Makes an object ready for the store and gives its identity.
fn prepare<T: Object>(object: &mut T) -> Result<ObjectKey, CoreError> {
    strip(object.meta_mut());
    ObjectKey::of(object.meta()).ok_or_else(no_name)
}

fn no_name() -> CoreError {
    CoreError::invalid("an object in the answer of the server has no name")
}

/// True for two copies of one version of one object. An object with no resource version
/// is never the same, so it always replaces the copy of the store.
fn same_version(known: &ObjectMeta, new: &ObjectMeta) -> bool {
    known.resource_version.is_some() && known.resource_version == new.resource_version && known.uid == new.uid
}

#[cfg(test)]
mod tests;
