//! The owner index (plan section 9.8): the objects below a workload, and the pods behind
//! a Service.
//!
//! - Deployment to ReplicaSet to Pod, StatefulSet and DaemonSet to Pod, CronJob to Job to
//!   Pod: a child names its owners in `ownerReferences`, by their UID.
//! - Service to EndpointSlice: the slice has the name of its Service in a label.
//!   EndpointSlice to Pod: each endpoint names its pod.
//!
//! `follow` holds the write side.

mod follow;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use k8s_openapi::api::discovery::v1::EndpointSlice;

pub(super) use follow::OwnerIndexer;

use super::links::Links;
use super::task::{Answer, Index};
use crate::kinds::{Kind, is_ready};
use crate::store::{Items, ObjectKey};

/// One object of the cluster of the index: its kind, its identity in the store of the
/// kind, and its UID.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ObjectRef {
    /// The kind of the object.
    pub kind: Kind,
    /// The namespace and the name.
    pub key: ObjectKey,
    /// The UID. An other object can get the same name later.
    pub uid: Arc<str>,
}

/// A pod behind a Service.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct EndpointPod {
    /// The pod.
    pub pod: ObjectKey,
    /// True when the pod takes new connections: it is ready and does not end. A port
    /// forward to a Service selects such a pod.
    pub ready: bool,
}

/// The relations of the owner index.
#[derive(Clone, Default)]
pub struct Owners {
    /// From the UID of an owner to each ReplicaSet, Job and Pod that names it as an owner.
    children: Links<Arc<str>, ObjectRef>,
    /// From a Service to its EndpointSlices.
    slices: Links<ObjectKey, ObjectKey>,
    /// The EndpointSlices of the last read. The index reads the pods of a Service from
    /// them, so the slices and the relation to them are of one moment.
    endpoints: Items<EndpointSlice>,
}

/// The stores that the objects below an owner of this kind come from, in the sequence
/// from the owner down. An answer is good only for what these stores hold.
fn below(owner: Kind) -> &'static [Kind] {
    match owner {
        Kind::Deployment => &[Kind::ReplicaSet, Kind::Pod],
        Kind::CronJob => &[Kind::Job, Kind::Pod],
        _ => &[Kind::Pod],
    }
}

impl Index<Owners> {
    /// The objects that name this owner in their `ownerReferences`: the ReplicaSets of a
    /// Deployment, the Jobs of a CronJob, the Pods of each other workload.
    pub fn owned(&self, owner: Kind, uid: &str) -> Answer<Vec<ObjectRef>> {
        let found = self.relations().children.of(uid).cloned().collect();
        Answer { found, basis: self.basis(&below(owner)[..1]) }
    }

    /// Each pod below this owner, at each depth: the pods of a ReplicaSet, of a
    /// Deployment, of a CronJob. With no pod, the basis of the answer tells if the
    /// cluster has none or if the pods are not loaded.
    ///
    /// The walk visits each object one time. Bad data of a cluster can hold two objects
    /// that own each other, and a walk with no memory would never end then.
    pub fn pods_of(&self, owner: Kind, uid: &str) -> Answer<Vec<ObjectRef>> {
        let children = &self.relations().children;
        let (mut pods, mut visited, mut open) = (BTreeSet::new(), BTreeSet::new(), vec![uid]);
        while let Some(owner) = open.pop() {
            if !visited.insert(owner) {
                continue;
            }
            for child in children.of(owner) {
                if child.kind == Kind::Pod {
                    pods.insert(child.clone());
                } else {
                    open.push(&child.uid);
                }
            }
        }
        Answer { found: pods.into_iter().collect(), basis: self.basis(below(owner)) }
    }

    /// The EndpointSlices of a Service.
    pub fn slices_of(&self, service: &ObjectKey) -> Answer<Vec<ObjectKey>> {
        let found = self.relations().slices.of(service).cloned().collect();
        Answer { found, basis: self.basis(&[Kind::EndpointSlice]) }
    }

    /// The pods behind a Service, as its EndpointSlices name them. A pod in two slices
    /// is one entry, and it is ready when one slice tells so.
    pub fn pods_of_service(&self, service: &ObjectKey) -> Answer<Vec<EndpointPod>> {
        let relations = self.relations();
        let mut pods = BTreeMap::new();
        let slices = relations.slices.of(service).filter_map(|slice| relations.endpoints.get(slice));
        for endpoint in slices.flat_map(|slice| &slice.endpoints) {
            let target = endpoint.target_ref.as_ref().filter(|target| target.kind.as_deref() == Some("Pod"));
            let Some((target, name)) = target.and_then(|target| Some((target, target.name.as_deref()?))) else {
                continue;
            };
            let pod = ObjectKey::new(target.namespace.as_deref().unwrap_or(&service.namespace), name);
            *pods.entry(pod).or_insert(false) |= is_ready(endpoint);
        }
        let found = pods.into_iter().map(|(pod, ready)| EndpointPod { pod, ready }).collect();
        Answer { found, basis: self.basis(&[Kind::EndpointSlice]) }
    }
}

#[cfg(test)]
mod tests;
