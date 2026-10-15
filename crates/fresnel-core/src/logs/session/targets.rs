//! The streams that a view wants now: one for each chosen container of each of its pods.
//!
//! A pod view reads its pod from the store of the pods. A workload view reads its pods
//! from the owner index, which tells "no pods" from "pods not loaded".

use std::sync::Arc;

use k8s_openapi::api::core::v1::Pod;

use crate::index::{Index, ObjectRef, Owners};
use crate::kinds::Kind;
use crate::logs::container;
use crate::logs::request::{Containers, LogRequest};
use crate::logs::view::PodsState;
use crate::store::{ObjectKey, Snapshot};

/// One stream that a view wants.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Want {
    pub(super) pod: ObjectKey,
    /// The UID of the pod: a new pod of the same name is an other source.
    pub(super) uid: Arc<str>,
    pub(super) container: Arc<str>,
}

/// The streams that a view wants now, and how far it knows its pods.
pub(super) struct Wanted {
    pub(super) pods: PodsState,
    pub(super) streams: Vec<Want>,
}

impl Wanted {
    fn none(pods: PodsState) -> Self {
        Self { pods, streams: Vec::new() }
    }
}

/// The streams of `request`. The default container is the same for the life of a view: the
/// first time that the pods are known decides it into `default`.
pub(super) fn wanted(
    request: &LogRequest,
    pods: &Snapshot<Pod>,
    owners: &Index<Owners>,
    default: &mut Option<Arc<str>>,
) -> Wanted {
    let chosen: Vec<&Arc<Pod>> = if request.kind == Kind::Pod {
        if !pods.listed {
            return Wanted::none(PodsState::Loading);
        }
        let same = |pod: &&Arc<Pod>| request.uid.is_empty() || pod.metadata.uid.as_deref() == Some(&*request.uid);
        match pods.items.get(&request.key).filter(same) {
            Some(pod) => vec![pod],
            None => return Wanted::none(PodsState::Gone),
        }
    } else {
        let answer = owners.pods_of(request.kind, &request.uid);
        if !answer.basis.is_loaded() {
            return Wanted::none(PodsState::Loading);
        }
        // The index can be some milliseconds ahead of the snapshot of the store: a pod that
        // the snapshot does not have yet comes with the next one.
        let in_store = |owned: &ObjectRef| {
            pods.items.get(&owned.key).filter(|pod| pod.metadata.uid.as_deref() == Some(&*owned.uid))
        };
        answer.found.iter().filter_map(in_store).collect()
    };
    if request.options.containers == Containers::Default && default.is_none() {
        let objects: Vec<&Pod> = chosen.iter().map(|pod| &***pod).collect();
        *default = container::default_of(&objects);
    }
    let mut streams = Vec::new();
    for pod in chosen {
        let Some(key) = ObjectKey::of(&pod.metadata) else {
            continue;
        };
        let uid: Arc<str> = pod.metadata.uid.as_deref().unwrap_or_default().into();
        for container in container::chosen(pod, &request.options.containers, default.as_ref()) {
            streams.push(Want { pod: key.clone(), uid: Arc::clone(&uid), container });
        }
    }
    Wanted { pods: PodsState::Following, streams }
}
