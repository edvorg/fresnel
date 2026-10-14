//! The objects of the kind tests, and a cluster with them. The Nodes, Deployments and
//! Services come from the scene of the indexes, as the pods and the indexes of each cluster
//! of the fake core, so their related objects are there. The other kinds come from the
//! scene of the kinds test, in the namespace of the pods.

use std::sync::Arc;

use fresnel_core::app::ClusterRow;
use fresnel_core::kinds::{Describe, Kind};
use fresnel_core::store::{Snapshot, StoreState, StoreStatus};
use k8s_openapi::api::apps::v1::{ControllerRevision, Deployment};
use k8s_openapi::api::core::v1::{ConfigMap, Node, Service, ServiceAccount};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::data::{data, held, of_kinds, pods, scene, store};
use crate::fake::{connected, live_status};

pub(crate) fn nodes() -> Vec<Arc<Node>> {
    scene()
}

pub(crate) fn deployments() -> Vec<Arc<Deployment>> {
    scene()
}

pub(crate) fn config_maps() -> Vec<Arc<ConfigMap>> {
    in_scene(of_kinds())
}

pub(crate) fn services() -> Vec<Arc<Service>> {
    scene()
}

pub(crate) fn service_accounts() -> Vec<Arc<ServiceAccount>> {
    in_scene(of_kinds())
}

pub(crate) fn revisions() -> Vec<Arc<ControllerRevision>> {
    in_scene(of_kinds())
}

/// The objects in the namespace `scene`, where the pods that use them are.
fn in_scene<K: Serialize + DeserializeOwned>(objects: Vec<Arc<K>>) -> Vec<Arc<K>> {
    let moved = |object: Arc<K>| {
        let mut value = serde_json::to_value(&*object).expect("JSON");
        value["metadata"]["namespace"] = "scene".into();
        Arc::new(serde_json::from_value(value).expect("an object"))
    };
    objects.into_iter().map(moved).collect()
}

/// A selected cluster with live connections, the pods and the indexes of the scene, and
/// this store of a kind, as the core gives it to a view that shows the kind.
/// The facts of its stores give the sidebar the counts of the pods and of the kind.
pub(crate) fn with<K: Describe>(name: &str, objects: Snapshot<K>) -> ClusterRow {
    let mut data = data(store(StoreState::Live, &pods()));
    let status = |kind, items| StoreStatus {
        kind,
        state: objects.state.clone(),
        since: objects.since,
        last_contact: objects.last_contact,
        listed: objects.listed,
        items,
        dropped: 0,
        generation: objects.generation,
    };
    data.stores = vec![status(Kind::Pod, data.pods.items.len()), status(K::ID, objects.items.len())].into();
    data.objects = data.objects.with(held(objects));
    ClusterRow { data: Some(data), ..connected(name, live_status()) }
}

/// A cluster with a live store of these objects.
pub(crate) fn live<K: Describe>(name: &str, objects: &[Arc<K>]) -> ClusterRow {
    with(name, store(StoreState::Live, objects))
}

/// The UID of the object with this name.
pub(crate) fn uid_of<K: Describe>(objects: &[Arc<K>], name: &str) -> Arc<str> {
    let object = objects.iter().find(|object| object.metadata().name.as_deref() == Some(name)).expect("the object");
    object.metadata().uid.as_deref().expect("a UID").into()
}
