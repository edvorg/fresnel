//! The editor model of a key view (plan section 12.2): the changes of the user on one read
//! of the object, the save of the changed keys only, and the conflicts with a new read.
//!
//! A change holds the new bytes of a key. A change back to the bytes of the read is no
//! change, so a save sends only the keys whose bytes differ (research 08 section 8.3).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Display};
use std::sync::Arc;

use super::name::key_problem;
use super::object::{KeyKind, KeyObject, Place, Target};
use super::special::{self, Special};
use super::value::Value;

/// The change of one key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    /// The key has this value: a new value, or a new key.
    Set(Value),
    /// The key goes away.
    Removed,
}

/// What a view shows of one key of a draft.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyState {
    /// The value of the read.
    Same,
    /// A new value.
    Changed,
    /// A key that the read does not have.
    Added,
    /// A key of the read that goes away.
    Removed,
}

/// One key of a draft, for the list of a view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DraftKey<'a> {
    /// The name of the key.
    pub name: &'a str,
    /// Its state.
    pub state: KeyState,
    /// The value of the draft; the value of the read for a key that goes away.
    pub value: &'a Value,
}

/// Why a draft refuses an edit or a save.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The object is immutable: the server refuses each change of a value.
    Immutable,
    /// The draft has no key of this name.
    NoSuchKey(String),
    /// The draft has a key of this name.
    KeyExists(String),
    /// The server refuses the name, for this reason.
    BadName(String),
    /// The draft has no change.
    NothingToSave,
    /// A new read is of an other object: an other cluster, kind, name or UID.
    OtherObject,
}

impl Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Immutable => f.write_str("the object is immutable: only a delete and a new create change it"),
            Self::NoSuchKey(key) => write!(f, "no key `{key}`"),
            Self::KeyExists(key) => write!(f, "the key `{key}` exists"),
            Self::BadName(reason) => f.write_str(reason),
            Self::NothingToSave => f.write_str("no key changed"),
            Self::OtherObject => f.write_str("the read is of an other object"),
        }
    }
}

/// A key that the user changed and that the server changed too, after the read of the
/// draft. The draft keeps the change of the user: a save sends it, and an undo of the key
/// takes the value of the server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    /// The name of the key.
    pub key: String,
    /// The value of the read of the draft. `None`: the read had no such key.
    pub base: Option<Value>,
    /// The value of the user. `None`: the user removed the key.
    pub mine: Option<Value>,
    /// The value of the server now. `None`: the server has no such key.
    pub server: Option<Value>,
}

/// The write of one key in a save.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyWrite {
    /// The name of the key.
    pub key: String,
    /// The map and the new value. `None` for a key that goes away.
    pub new: Option<(Place, Value)>,
    /// The map of the key in the read. `None` for a new key.
    pub old: Option<Place>,
}

/// The save of a draft: the changed keys, with the resource version of the read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Save {
    /// The object, with the UID of the read.
    pub target: Target,
    /// The resource version of the read.
    pub version: Arc<str>,
    /// Each changed key, in the sequence of the names.
    pub writes: Vec<KeyWrite>,
}

/// The changes of the user on one read of a Secret or a ConfigMap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Draft {
    base: Arc<KeyObject>,
    changes: BTreeMap<String, Change>,
}

impl Draft {
    /// A draft with no change.
    pub fn new(base: Arc<KeyObject>) -> Self {
        Self { base, changes: BTreeMap::new() }
    }

    /// The read that the draft changes.
    pub fn base(&self) -> &Arc<KeyObject> {
        &self.base
    }

    /// Each change, by the name of its key.
    pub fn changes(&self) -> &BTreeMap<String, Change> {
        &self.changes
    }

    /// True when a save has something to send.
    pub fn is_changed(&self) -> bool {
        !self.changes.is_empty()
    }

    /// The value of a key in the draft. `None` for a key that goes away or that does not exist.
    pub fn value(&self, key: &str) -> Option<&Value> {
        match self.changes.get(key) {
            Some(Change::Set(value)) => Some(value),
            Some(Change::Removed) => None,
            None => self.base.value(key),
        }
    }

    /// Each key of the read and each new key, in the sequence of the names.
    pub fn keys(&self) -> Vec<DraftKey<'_>> {
        let names: BTreeSet<&str> = self.base.entries.keys().chain(self.changes.keys()).map(String::as_str).collect();
        names.into_iter().filter_map(|name| self.key(name)).collect()
    }

    fn key<'a>(&'a self, name: &'a str) -> Option<DraftKey<'a>> {
        let read = self.base.value(name);
        let (state, value) = match (self.changes.get(name), read) {
            (None, Some(value)) => (KeyState::Same, value),
            (Some(Change::Set(value)), Some(_)) => (KeyState::Changed, value),
            (Some(Change::Set(value)), None) => (KeyState::Added, value),
            (Some(Change::Removed), Some(value)) => (KeyState::Removed, value),
            (None | Some(Change::Removed), None) => return None,
        };
        Some(DraftKey { name, state, value })
    }

    /// Gives an existing key a new value.
    pub fn set(&mut self, key: &str, bytes: impl Into<Arc<[u8]>>) -> Result<(), Refusal> {
        self.editable()?;
        if self.value(key).is_none() {
            return Err(Refusal::NoSuchKey(key.to_owned()));
        }
        self.put(key, Value::new(bytes));
        Ok(())
    }

    /// Adds a key. A key that goes away can come back with this call.
    pub fn add(&mut self, key: &str, bytes: impl Into<Arc<[u8]>>) -> Result<(), Refusal> {
        self.editable()?;
        if let Some(problem) = key_problem(key) {
            return Err(Refusal::BadName(problem));
        }
        if self.value(key).is_some() {
            return Err(Refusal::KeyExists(key.to_owned()));
        }
        self.put(key, Value::new(bytes));
        Ok(())
    }

    /// Removes a key.
    pub fn remove(&mut self, key: &str) -> Result<(), Refusal> {
        self.editable()?;
        if self.value(key).is_none() {
            return Err(Refusal::NoSuchKey(key.to_owned()));
        }
        if self.base.entries.contains_key(key) {
            self.changes.insert(key.to_owned(), Change::Removed);
        } else {
            self.changes.remove(key);
        }
        Ok(())
    }

    /// Drops the change of a key: the key has the value of the read again.
    pub fn undo(&mut self, key: &str) {
        self.changes.remove(key);
    }

    /// The save of the changes. A key of a ConfigMap stays in its map while its value fits
    /// there: a move changes where a client finds it. A value that is not UTF-8 goes to
    /// `binaryData`, and a new key that is UTF-8 to `data`.
    pub fn save(&self) -> Result<Save, Refusal> {
        self.editable()?;
        if self.changes.is_empty() {
            return Err(Refusal::NothingToSave);
        }
        let kind = self.base.target.kind;
        let write = |(key, change): (&String, &Change)| {
            let old = self.base.entries.get(key).map(|entry| entry.place);
            let new = match change {
                Change::Set(value) => Some((place(kind, old, value), value.clone())),
                Change::Removed => None,
            };
            KeyWrite { key: key.clone(), new, old }
        };
        let writes = self.changes.iter().map(write).collect();
        Ok(Save { target: self.base.target.clone(), version: Arc::clone(&self.base.version), writes })
    }

    /// Puts the changes of the user onto a new read of the same object, for example after
    /// the server refused a save with a conflict. A change that the server has now is no
    /// change any more. Gives each key that the user and the server both changed; the draft
    /// keeps the change of the user for it.
    pub fn rebase(&mut self, server: Arc<KeyObject>) -> Result<Vec<Conflict>, Refusal> {
        if server.target != self.base.target {
            return Err(Refusal::OtherObject);
        }
        let mut conflicts = Vec::new();
        let changes = std::mem::take(&mut self.changes);
        for (key, change) in changes {
            let (base, now) = (self.base.value(&key), server.value(&key));
            let mine = match &change {
                Change::Set(value) => Some(value),
                Change::Removed => None,
            };
            if bytes(mine) == bytes(now) {
                continue;
            }
            if bytes(base) != bytes(now) {
                let (base, mine, server) = (base.cloned(), mine.cloned(), now.cloned());
                conflicts.push(Conflict { key: key.clone(), base, mine, server });
            }
            self.changes.insert(key, change);
        }
        self.base = server;
        Ok(conflicts)
    }

    /// The special view of the values of the draft.
    pub fn special(&self) -> Special {
        let value = |key: &str| self.value(key).map(Value::bytes);
        self.base.secret_type.as_ref().map_or(Special::None, |kind| special::of(kind, value))
    }

    fn editable(&self) -> Result<(), Refusal> {
        if self.base.immutable { Err(Refusal::Immutable) } else { Ok(()) }
    }

    /// Records a value. The bytes of the read are no change.
    fn put(&mut self, key: &str, value: Value) {
        if self.base.value(key).is_some_and(|read| read.bytes() == value.bytes()) {
            self.changes.remove(key);
        } else {
            self.changes.insert(key.to_owned(), Change::Set(value));
        }
    }
}

fn bytes(value: Option<&Value>) -> Option<&[u8]> {
    value.map(Value::bytes)
}

/// The map of a key after a save: `data` for each key of a Secret, the map of the read for
/// a ConfigMap value that fits there, `binaryData` for one that is not UTF-8.
fn place(kind: KeyKind, old: Option<Place>, value: &Value) -> Place {
    match kind {
        KeyKind::Secret => Place::Data,
        KeyKind::ConfigMap if std::str::from_utf8(value.bytes()).is_err() => Place::BinaryData,
        KeyKind::ConfigMap => old.unwrap_or(Place::Data),
    }
}

#[cfg(test)]
mod tests;
