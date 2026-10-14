//! The full objects of the kinds that the views show (plan section 9.5).
//!
//! A view names its kinds with `Command::ShowKinds`. The core then holds a `Demand` of each
//! such kind in each connected cluster: the store of a tier B or tier C kind runs while a
//! view shows the kind, and the view reads its snapshot in `ClusterData::objects`.

use std::any::Any;
use std::collections::BTreeSet;
use std::sync::Arc;

use crate::kinds::{Describe, Kind, Visit};
use crate::published::Held;
use crate::store::{ClusterStores, Demand, Snapshot};

/// The snapshots of the full objects of some kinds of one cluster, at one moment. A view
/// gets the snapshot of a kind with the type of its objects.
#[derive(Clone, Default)]
pub struct Objects {
    /// The `Held<Snapshot<K>>` of each kind, with its count of objects.
    snapshots: Vec<(Kind, usize, Arc<dyn Any + Send + Sync>)>,
}

impl Objects {
    /// The same objects and this snapshot, in place of an older one of its kind.
    pub fn with<K: Describe>(mut self, snapshot: Held<Snapshot<K>>) -> Self {
        self.snapshots.retain(|(kind, ..)| *kind != K::ID);
        self.snapshots.push((K::ID, snapshot.items.len(), Arc::new(snapshot)));
        self
    }

    /// The snapshot of the full objects of a kind. `None` for a kind that no view showed.
    pub fn get<K: Describe>(&self) -> Option<&Held<Snapshot<K>>> {
        let (.., snapshot) = self.snapshots.iter().find(|(kind, ..)| *kind == K::ID)?;
        snapshot.downcast_ref()
    }

    /// The count of the objects of a kind. `None` for a kind that no view showed.
    pub fn count(&self, kind: Kind) -> Option<usize> {
        self.snapshots.iter().find(|(of, ..)| *of == kind).map(|(_, count, _)| *count)
    }

    /// The kinds of the snapshots.
    pub fn kinds(&self) -> impl Iterator<Item = Kind> + '_ {
        self.snapshots.iter().map(|(kind, ..)| *kind)
    }
}

impl std::fmt::Debug for Objects {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.kinds()).finish()
    }
}

/// The holder of the store of one kind, with no type of its objects.
trait Shown: Send + Sync {
    fn kind(&self) -> Kind;

    /// Adds the snapshot of now to `objects`.
    fn read_into(&self, objects: Objects) -> Objects;
}

impl<K: Describe> Shown for Demand<K> {
    fn kind(&self) -> Kind {
        K::ID
    }

    fn read_into(&self, objects: Objects) -> Objects {
        objects.with(self.read())
    }
}

/// The holders of the stores of the shown kinds of one connected cluster.
#[derive(Default)]
pub(super) struct ShownKinds {
    holders: Vec<Box<dyn Shown>>,
}

impl ShownKinds {
    /// Holds the store of each kind of `kinds` and lets the other ones go. A store that a
    /// view shows again in its time after the last holder needs no new list.
    pub(super) fn set(&mut self, stores: &ClusterStores, kinds: &BTreeSet<Kind>) {
        self.holders.retain(|holder| kinds.contains(&holder.kind()));
        for kind in kinds {
            if !self.holders.iter().any(|holder| holder.kind() == *kind)
                && let Some(holder) = kind.visit(Hold(stores))
            {
                self.holders.push(holder);
            }
        }
    }

    /// The snapshot of each held kind. The call copies one pointer for each kind.
    pub(super) fn read(&self) -> Objects {
        self.holders.iter().fold(Objects::default(), |objects, holder| holder.read_into(objects))
    }
}

/// Starts the holder of the store of one kind.
struct Hold<'a>(&'a ClusterStores);

impl Visit for Hold<'_> {
    type Output = Option<Box<dyn Shown>>;

    fn visit<K: Describe>(self) -> Self::Output {
        Some(Box::new(self.0.demand::<K>()?))
    }
}

#[cfg(test)]
mod tests {
    use k8s_openapi::api::core::v1::{ConfigMap, Pod};

    use super::*;
    use crate::published::{Disposal, Notice, Publisher};
    use crate::store::StoreState;

    #[tokio::test]
    async fn a_view_gets_the_snapshot_of_a_kind_with_the_type_of_its_objects_only() {
        let tasks = fresnel_net::task::Tasks::new(tokio::runtime::Handle::current(), |_| {});
        let disposal = Disposal::start(&tasks);
        let snapshot = |state| Snapshot::<ConfigMap>::empty(state, std::time::SystemTime::UNIX_EPOCH);
        let held = |state| Publisher::new(snapshot(state), disposal.bin(), Notice::new()).read();

        let objects = Objects::default().with(held(StoreState::Live));
        assert_eq!(objects.get::<ConfigMap>().map(|snapshot| snapshot.state.clone()), Some(StoreState::Live));
        assert!(objects.get::<Pod>().is_none(), "no view showed the pods");
        let objects = objects.with(held(StoreState::Stopped));
        assert_eq!(objects.kinds().collect::<Vec<_>>(), [Kind::ConfigMap], "one snapshot of a kind");
        assert_eq!(objects.get::<ConfigMap>().map(|snapshot| snapshot.state.clone()), Some(StoreState::Stopped));
        assert_eq!((objects.count(Kind::ConfigMap), objects.count(Kind::Pod)), (Some(0), None));
    }
}
