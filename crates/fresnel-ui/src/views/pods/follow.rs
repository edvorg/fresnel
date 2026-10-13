//! Follows the Pod stores of the selected clusters from frame to frame.
//!
//! The view asks its worker for new rows only when the objects changed, and it must drop
//! each deleted pod from its selection. The differences between the snapshot of the last
//! frame and the snapshot of this one give the two answers. Their cost follows the count
//! of the changes: two frames with the same objects cost one comparison of two pointers.

use std::collections::HashSet;

use fresnel_core::cluster::ClusterId;
use fresnel_core::published::Held;
use fresnel_core::store::{Change, Snapshot};
use k8s_openapi::api::core::v1::Pod;

use super::rows::PodData;
use crate::widgets::table::model::ObjectId;

/// The objects that went away from one frame to the next.
#[derive(Debug, Default)]
pub(super) struct Gone {
    /// The clusters that left the selection, or whose connections closed.
    clusters: Vec<ClusterId>,
    /// The pods that left the store of their cluster.
    pods: HashSet<ObjectId>,
}

impl Gone {
    pub(super) fn is_empty(&self) -> bool {
        self.clusters.is_empty() && self.pods.is_empty()
    }

    pub(super) fn has(&self, id: &ObjectId) -> bool {
        self.clusters.contains(&id.cluster) || self.pods.contains(id)
    }
}

/// The snapshots of the last frame, and a number that grows when the objects change.
///
/// A snapshot that this value held goes to the thread that frees old snapshots: the UI
/// thread never frees one (plan section 7.3).
#[derive(Default)]
pub(super) struct Follower {
    seen: Vec<(ClusterId, Held<Snapshot<Pod>>)>,
    serial: u64,
}

fn uid(pod: &Pod) -> &str {
    pod.metadata.uid.as_deref().unwrap_or_default()
}

impl Follower {
    /// Takes the snapshots of this frame and gives the objects that went away.
    pub(super) fn step(&mut self, now: Vec<(ClusterId, Held<Snapshot<Pod>>)>) -> Gone {
        let mut gone = Gone::default();
        let mut changed = now.len() != self.seen.len();
        for (cluster, before) in &self.seen {
            let Some((_, after)) = now.iter().find(|(id, _)| id == cluster) else {
                gone.clusters.push(cluster.clone());
                changed = true;
                continue;
            };
            for change in before.items.changes_to(&after.items) {
                changed = true;
                // A new pod with the name of an old one is an other object.
                let left = match change {
                    Change::Removed(_, old) => Some(old),
                    Change::Updated { old, new, .. } if uid(old) != uid(new) => Some(old),
                    Change::Added(..) | Change::Updated { .. } => None,
                };
                if let Some(old) = left {
                    gone.pods.insert(ObjectId { cluster: cluster.clone(), uid: uid(old).into() });
                }
            }
        }
        self.seen = now;
        self.serial += u64::from(changed);
        gone
    }

    /// A number that grows each time the objects of a step differ from the step before.
    pub(super) fn serial(&self) -> u64 {
        self.serial
    }

    /// The snapshots of the last step, for a request to the worker.
    pub(super) fn data(&self) -> PodData {
        PodData { clusters: self.seen.clone() }
    }

    /// The snapshot of one cluster at the last step.
    pub(super) fn snapshot(&self, cluster: &ClusterId) -> Option<&Snapshot<Pod>> {
        self.seen.iter().find(|(id, _)| id == cluster).map(|(_, snapshot)| &**snapshot)
    }
}
