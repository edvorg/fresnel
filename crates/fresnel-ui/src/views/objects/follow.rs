//! Follows the stores of one kind of the selected clusters from frame to frame, and the
//! metrics of a kind with the CPU and memory columns.
//!
//! The page asks its worker for new rows only when the objects or the metrics changed, and
//! it must drop each deleted object from its selection. The differences between the snapshot of the last
//! frame and the snapshot of this one give the two answers. Their cost follows the count
//! of the changes: two frames with the same objects cost one comparison of two pointers.

use std::collections::HashSet;

use fresnel_core::cluster::ClusterId;
use fresnel_core::kinds::Object;
use fresnel_core::published::Held;
use fresnel_core::store::{Change, Snapshot};

use super::{Stores, uid};
use crate::views::metrics::read::Metered;
use crate::widgets::table::model::ObjectId;

/// The objects that went away from one frame to the next.
#[derive(Debug, Default)]
pub(super) struct Gone {
    /// The clusters that left the selection, or whose connections closed.
    clusters: Vec<ClusterId>,
    /// The objects that left the store of their cluster.
    objects: HashSet<ObjectId>,
}

impl Gone {
    pub(super) fn is_empty(&self) -> bool {
        self.clusters.is_empty() && self.objects.is_empty()
    }

    pub(super) fn has(&self, id: &ObjectId) -> bool {
        self.clusters.contains(&id.cluster) || self.objects.contains(id)
    }
}

/// The snapshots and the metrics of the last frame, and a number that grows when they
/// change.
///
/// A snapshot that this value held goes to the thread that frees old snapshots: the UI
/// thread never frees one (plan section 7.3).
pub(super) struct Follower<T: Object> {
    seen: Vec<(ClusterId, Held<Snapshot<T>>)>,
    metered: Vec<Metered>,
    serial: u64,
}

impl<T: Object> Default for Follower<T> {
    fn default() -> Self {
        Self { seen: Vec::new(), metered: Vec::new(), serial: 0 }
    }
}

impl<T: Object> Follower<T> {
    /// Takes the snapshots and the metrics of this frame and gives the objects that went
    /// away. A new poll of the metrics is a change: the rows have its values.
    pub(super) fn step(&mut self, now: Vec<(ClusterId, Held<Snapshot<T>>)>, metered: Vec<Metered>) -> Gone {
        let mut gone = Gone::default();
        let same_polls = metered.len() == self.metered.len()
            && metered.iter().zip(&self.metered).all(|(now, before)| now.is_same(before));
        let mut changed = now.len() != self.seen.len() || !same_polls;
        for (cluster, before) in &self.seen {
            let Some((_, after)) = now.iter().find(|(id, _)| id == cluster) else {
                gone.clusters.push(cluster.clone());
                changed = true;
                continue;
            };
            for change in before.items.changes_to(&after.items) {
                changed = true;
                // A new object with the name of an old one is an other object.
                let left = match change {
                    Change::Removed(key, old) => Some((key, old)),
                    Change::Updated { key, old, new } if uid(old.meta()) != uid(new.meta()) => Some((key, old)),
                    Change::Added(..) | Change::Updated { .. } => None,
                };
                if let Some((key, old)) = left {
                    let id = ObjectId { cluster: cluster.clone(), key: key.clone(), uid: uid(old.meta()).into() };
                    gone.objects.insert(id);
                }
            }
        }
        (self.seen, self.metered) = (now, metered);
        self.serial += u64::from(changed);
        gone
    }

    /// Gives up the snapshots, for a view that is not painted. The next step gets a new
    /// number: the number is a part of the identity of a request for rows, and the objects
    /// after a rest are not the objects before it.
    pub(super) fn rest(&mut self) {
        if !self.seen.is_empty() || !self.metered.is_empty() {
            self.seen.clear();
            self.metered.clear();
            self.serial += 1;
        }
    }

    /// A number that grows each time the objects of a step differ from the step before.
    pub(super) fn serial(&self) -> u64 {
        self.serial
    }

    /// The snapshots of the last step, for a request to the worker.
    pub(super) fn data(&self) -> Stores<T> {
        Stores { clusters: self.seen.clone(), metered: self.metered.clone() }
    }

    /// The snapshot of one cluster at the last step.
    pub(super) fn snapshot(&self, cluster: &ClusterId) -> Option<&Snapshot<T>> {
        self.seen.iter().find(|(id, _)| id == cluster).map(|(_, snapshot)| &**snapshot)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fresnel_core::store::{ObjectKey, StoreState};
    use k8s_openapi::api::core::v1::Pod;

    use super::*;
    use crate::testing::{TestTasks, fixture_pods, snapshot};

    fn step(bed: &TestTasks, follower: &mut Follower<Pod>, clusters: &[(&str, &[Arc<Pod>])]) -> Gone {
        let held =
            |(name, pods): &(&str, &[Arc<Pod>])| (ClusterId::new(*name), bed.held(snapshot(pods, StoreState::Live)));
        follower.step(clusters.iter().map(held).collect(), Vec::new())
    }

    fn id(cluster: &str, pod: &Pod) -> ObjectId {
        let key = ObjectKey::new("scene", pod.metadata.name.as_deref().expect("a name"));
        ObjectId { cluster: ClusterId::new(cluster), key, uid: uid(&pod.metadata).into() }
    }

    #[test]
    fn a_step_gives_the_pods_and_the_clusters_that_went_away_and_a_new_number_for_new_objects() {
        let bed = TestTasks::new();
        let pods: Vec<Arc<Pod>> =
            fixture_pods().into_iter().filter(|pod| pod.metadata.namespace.as_deref() == Some("scene")).collect();
        let mut follower = Follower::default();
        assert!(step(&bed, &mut follower, &[("ams", &pods), ("fra", &pods)]).is_empty());
        let first = follower.serial();

        // A store publishes the same objects again for a new state: no new number.
        assert!(step(&bed, &mut follower, &[("ams", &pods), ("fra", &pods)]).is_empty());
        assert_eq!(follower.serial(), first);

        // One pod of ams goes away, and a new pod has the name of an other one.
        let (deleted, replaced) = (Arc::clone(&pods[0]), Arc::clone(&pods[1]));
        let mut next = Pod::clone(&replaced);
        next.metadata.uid = Some("the-uid-of-a-new-pod".to_owned());
        let after: Vec<Arc<Pod>> = pods[2..].iter().cloned().chain([Arc::new(next.clone())]).collect();
        let gone = step(&bed, &mut follower, &[("ams", &after), ("fra", &pods)]);
        assert!(gone.has(&id("ams", &deleted)) && gone.has(&id("ams", &replaced)));
        assert!(!gone.has(&id("ams", &next)) && !gone.has(&id("ams", &pods[2])), "the pods that stay");
        assert!(!gone.has(&id("fra", &deleted)), "the same pod of an other cluster is an other object");
        assert!(follower.serial() > first);

        let before = follower.serial();
        let gone = step(&bed, &mut follower, &[("ams", &after)]);
        assert!(gone.has(&id("fra", &pods[3])) && !gone.has(&id("ams", &pods[3])), "each pod of fra went away");
        assert!(follower.serial() > before && follower.snapshot(&ClusterId::new("fra")).is_none());
    }

    /// The number is a part of the identity of a request for rows. With the number of
    /// before, a request after a rest could get the rows of a request before it.
    #[test]
    fn a_rest_gives_up_the_snapshots_and_the_objects_after_it_have_a_new_number() {
        let bed = TestTasks::new();
        let mut follower = Follower::default();
        step(&bed, &mut follower, &[("ams", &fixture_pods())]);
        let before = follower.serial();
        follower.rest();
        assert!(follower.snapshot(&ClusterId::new("ams")).is_none() && follower.data().clusters.is_empty());
        // No cluster is selected at the return: the step itself finds no difference.
        assert!(step(&bed, &mut follower, &[]).is_empty());
        assert!(follower.serial() > before);

        let at_rest = follower.serial();
        follower.rest();
        assert_eq!(follower.serial(), at_rest, "a rest with nothing to give up changes nothing");
    }
}
