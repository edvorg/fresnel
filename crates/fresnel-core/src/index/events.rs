//! The Event index: from an object to the Events about it (plan section 9.6).
//!
//! An Event names its object in `involvedObject`. The index finds the Events of an object
//! by the UID. Some sources write no UID of the API: the kubelet puts the name of its
//! node in the place of the UID. The index finds these Events by the kind and the name.
//!
//! `on_demand` reads the Events of one object from the server, for a cluster with no
//! store of the Events.

mod on_demand;

use std::cmp::Reverse;
use std::collections::BTreeSet;
use std::sync::Arc;

use k8s_openapi::api::core::v1::Event;
use k8s_openapi::jiff::Timestamp;

pub use on_demand::{EventsRead, read_events};

use super::links::Links;
use super::source::{SourceStatus, Tracked};
use super::task::{Answer, Index, IndexState, Indexer, Read};
use crate::kinds::Kind;
use crate::memory::Account;
use crate::published::Reader;
use crate::store::{Change, Items, ObjectKey, Snapshot};

/// The relations of the Event index.
#[derive(Clone, Default)]
pub struct ObjectEvents {
    /// From the UID of an object to the Events about it.
    by_uid: Links<Arc<str>, ObjectKey>,
    /// From the kind and the identity of an object to the Events that name it with no UID
    /// of the API.
    by_name: Links<(Kind, ObjectKey), ObjectKey>,
    /// The Events of the last read, so that an answer and its relation are of one moment.
    events: Items<Event>,
}

impl super::Measured for ObjectEvents {
    /// The Events are those of their store: they count, with no bytes.
    fn account(&self, index: &str, account: &mut Account) {
        super::links(&format!("{index} by UID"), &self.by_uid, account);
        super::links(&format!("{index} by name"), &self.by_name, account);
        account.add(&format!("{index} Events"), self.events.len(), 0);
    }
}

impl Index<ObjectEvents> {
    /// The Events about one object, the newest first. The store of the Events has a
    /// maximum count and can hold the warnings only: the answer has what the store has.
    pub fn events_of(&self, kind: Kind, key: &ObjectKey, uid: &str) -> Answer<Vec<Arc<Event>>> {
        let relations = self.relations();
        let named = relations.by_name.of(&(kind, key.clone()));
        let keys: BTreeSet<&ObjectKey> = relations.by_uid.of(uid).chain(named).collect();
        let mut found: Vec<(&ObjectKey, &Arc<Event>)> =
            keys.into_iter().filter_map(|key| Some((key, relations.events.get(key)?))).collect();
        found.sort_by_key(|(key, event)| (Reverse(last_seen(event)), *key));
        let found = found.into_iter().map(|(_, event)| Arc::clone(event)).collect();
        Answer { found, basis: self.basis(&[Kind::Event]) }
    }
}

impl Index<ObjectEvents> {
    /// The index of the Events of one snapshot, as the task of the index gives it after
    /// one read, with `state` as the state of that task. A test of a reader builds the
    /// index of each state with it.
    pub fn of_snapshot(events: &Snapshot<Event>, state: IndexState) -> Self {
        let mut relations = ObjectEvents { events: events.items.clone(), ..ObjectEvents::default() };
        for (key, event) in events.items.iter() {
            link(&mut relations, subject(event), key);
        }
        Self::following(1, vec![SourceStatus::of(Kind::Event, events)], relations).in_state(state)
    }
}

/// The last time that the Event occurred. A source that repeats an Event changes its
/// series or its last timestamp, and not the creation time of the object. The answers
/// of the index have this sequence, and a view shows this time.
pub fn last_seen(event: &Event) -> Option<Timestamp> {
    let series = event.series.as_ref().and_then(|series| series.last_observed_time.as_ref()).map(|time| time.0);
    let last = event.last_timestamp.as_ref().map(|time| time.0);
    let first = event.event_time.as_ref().map(|time| time.0);
    series.or(last).or(first).or(event.metadata.creation_timestamp.as_ref().map(|time| time.0))
}

/// How the index finds the Events of one object.
#[derive(PartialEq, Eq)]
enum Subject {
    Uid(Arc<str>),
    Name(Kind, ObjectKey),
}

/// The object of an Event. `None` for an Event that names no object that the app can
/// show: one with no UID of the API and a kind that is out of scope.
fn subject(event: &Event) -> Option<Subject> {
    let object = &event.involved_object;
    let name = object.name.as_deref().unwrap_or_default();
    match object.uid.as_deref().filter(|uid| !uid.is_empty() && *uid != name) {
        Some(uid) => Some(Subject::Uid(uid.into())),
        None => {
            let kind = Kind::named(object.kind.as_deref()?)?;
            Some(Subject::Name(kind, ObjectKey::new(object.namespace.as_deref().unwrap_or_default(), name)))
        }
    }
}

pub(super) struct EventIndexer {
    relations: ObjectEvents,
    events: Tracked<Event>,
}

impl EventIndexer {
    pub(super) fn new(events: Reader<Snapshot<Event>>) -> Self {
        Self { relations: ObjectEvents::default(), events: Tracked::new(Kind::Event, events) }
    }
}

fn link(relations: &mut ObjectEvents, subject: Option<Subject>, event: &ObjectKey) {
    match subject {
        Some(Subject::Uid(uid)) => {
            relations.by_uid.add(uid, event.clone());
        }
        Some(Subject::Name(kind, key)) => {
            relations.by_name.add((kind, key), event.clone());
        }
        None => {}
    }
}

fn unlink(relations: &mut ObjectEvents, subject: Option<Subject>, event: &ObjectKey) {
    match subject {
        Some(Subject::Uid(uid)) => {
            relations.by_uid.remove(&*uid, event);
        }
        Some(Subject::Name(kind, key)) => {
            relations.by_name.remove(&(kind, key), event);
        }
        None => {}
    }
}

impl Indexer for EventIndexer {
    type Relations = ObjectEvents;

    /// Each change of an Event is a change of the index: an answer holds the Events
    /// themselves, with their counts and times.
    fn read(&mut self) -> Read {
        let relations = &mut self.relations;
        let mut changed = false;
        let source = self.events.read(|change| {
            changed = true;
            let (key, old, new) = match change {
                Change::Added(key, new) => (key, None, subject(new)),
                Change::Removed(key, old) => (key, subject(old), None),
                Change::Updated { key, old, new } => (key, subject(old), subject(new)),
            };
            if old != new {
                unlink(relations, old, key);
                link(relations, new, key);
            }
        });
        relations.events = self.events.seen().clone();
        Read { changed, sources: vec![source] }
    }

    fn relations(&self) -> ObjectEvents {
        self.relations.clone()
    }

    async fn changed(&mut self) {
        self.events.changed().await;
    }
}

#[cfg(test)]
mod tests;
