//! The reference index (plan section 9.9): from a ConfigMap, a Secret, a ServiceAccount
//! or a Node to the pods that use it.
//!
//! A pod names what it uses by name, so the index needs no store of these kinds: the
//! view of a ConfigMap asks with the identity of its object. A reference needs no object,
//! and the index holds it also for a ConfigMap that does not exist.
//!
//! `of_pod` reads the references of one pod.

mod of_pod;

use std::collections::BTreeSet;

use k8s_openapi::api::core::v1::Pod;

pub use of_pod::references_of;

use super::links::Links;
use super::source::{SourceStatus, Tracked};
use super::task::{Answer, Index, IndexState, Indexer, Read};
use crate::kinds::Kind;
use crate::memory::Account;
use crate::published::Reader;
use crate::store::{Change, Items, ObjectKey, Snapshot};

/// An object that a pod uses: its kind and its identity. The kind is `ConfigMap`,
/// `Secret`, `ServiceAccount` or `Node`. A Node has no namespace.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Target {
    /// The kind of the object.
    pub kind: Kind,
    /// The namespace and the name.
    pub key: ObjectKey,
}

/// The relations of the reference index.
#[derive(Clone, Default)]
pub struct References {
    /// From an object to each pod that uses it.
    users: Links<Target, ObjectKey>,
}

impl super::Measured for References {
    fn account(&self, index: &str, account: &mut Account) {
        super::links(&format!("{index} users"), &self.users, account);
    }
}

impl Index<References> {
    /// The pods that use this object: the "used by" of a ConfigMap or a Secret, the pods
    /// of a ServiceAccount, the pods on a Node.
    pub fn used_by(&self, kind: Kind, key: &ObjectKey) -> Answer<Vec<ObjectKey>> {
        let target = Target { kind, key: key.clone() };
        Answer { found: self.relations().users.of(&target).cloned().collect(), basis: self.basis(&[Kind::Pod]) }
    }

    /// The reference index of a snapshot of the pods, as the task of the index gives it
    /// after one read, with `state` as the state of that task. A test of a reader builds
    /// the index with it.
    pub fn of_pods(pods: &Snapshot<Pod>, state: IndexState) -> Self {
        let mut relations = References::default();
        for change in Items::default().changes_to(&pods.items) {
            follow_pod(&mut relations.users, change);
        }
        Self::following(1, vec![SourceStatus::of(Kind::Pod, pods)], relations).in_state(state)
    }
}

pub(super) struct ReferenceIndexer {
    relations: References,
    pods: Tracked<Pod>,
}

impl ReferenceIndexer {
    pub(super) fn new(pods: Reader<Snapshot<Pod>>) -> Self {
        Self { relations: References::default(), pods: Tracked::new(Kind::Pod, pods) }
    }
}

impl Indexer for ReferenceIndexer {
    type Relations = References;

    fn read(&mut self) -> Read {
        let users = &mut self.relations.users;
        let mut changed = false;
        let source = self.pods.read(|change| changed |= follow_pod(users, change));
        Read { changed, sources: vec![source] }
    }

    fn relations(&self) -> References {
        self.relations.clone()
    }

    async fn changed(&mut self) {
        self.pods.changed().await;
    }
}

/// Applies one change of the store of the pods. True when a relation changed: most
/// changes of a pod are a new status and leave its references as they are.
fn follow_pod(users: &mut Links<Target, ObjectKey>, change: Change<'_, Pod>) -> bool {
    let none = BTreeSet::new;
    let (key, old, new) = match change {
        Change::Added(key, new) => (key, none(), references_of(new)),
        Change::Removed(key, old) => (key, references_of(old), none()),
        // The references come from the spec, and a new status has the spec of before.
        Change::Updated { old, new, .. } if old.spec == new.spec => return false,
        Change::Updated { key, old, new } => (key, references_of(old), references_of(new)),
    };
    let mut changed = false;
    for gone in old.difference(&new) {
        changed |= users.remove(gone, key);
    }
    for added in new.difference(&old) {
        changed |= users.add(added.clone(), key.clone());
    }
    changed
}

#[cfg(test)]
mod tests;
