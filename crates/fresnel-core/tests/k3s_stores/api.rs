//! The API of the test cluster as the test itself uses it: with no proxy and no store.

use std::collections::BTreeMap;
use std::time::Duration;

use fresnel_core::cluster::Gateway;
use fresnel_core::kinds::{Full, Kind, Meta, Object, Tier};
use fresnel_core::published::Reader;
use fresnel_core::store::{ClusterStores, Snapshot};
use k8s_openapi::api::apps::v1::{DaemonSet, Deployment, ReplicaSet, StatefulSet};
use k8s_openapi::api::batch::v1::{CronJob, Job};
use k8s_openapi::api::core::v1::{Event, Namespace, Node, Pod, Service};
use k8s_openapi::api::discovery::v1::EndpointSlice;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ListMeta;
use kube::core::Request;
use kube::core::params::{DeleteParams, ListParams, Patch, PatchParams, PostParams};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::time::{Instant, sleep};

/// The resource version of each object of one kind, by namespace and name.
pub(crate) type Versions = BTreeMap<(String, String), String>;

/// A list with no object has `null` as its items.
#[derive(Deserialize)]
struct Page {
    metadata: ListMeta,
    items: Option<Vec<Meta>>,
}

fn version_of(meta: &Meta) -> ((String, String), String) {
    let text = |part: &Option<String>| part.clone().unwrap_or_default();
    let meta = &meta.metadata;
    ((text(&meta.namespace), text(&meta.name)), text(&meta.resource_version))
}

/// The objects of one kind, as a list of the API gives them now.
pub(crate) async fn versions(api: &Gateway, kind: Kind) -> Versions {
    let request = Request::new(kind.info().path());
    let (mut versions, mut params) = (Versions::new(), ListParams::default().limit(500));
    loop {
        let page: Page = api.list_page(request.list_metadata(&params).expect("a request")).await.expect("a list page");
        versions.extend(page.items.unwrap_or_default().iter().map(version_of));
        match page.metadata.continue_.filter(|token| !token.is_empty()) {
            Some(token) => params = params.continue_token(&token),
            None => return versions,
        }
    }
}

fn of_store<T: Object>(store: Option<Reader<Snapshot<T>>>) -> Versions {
    let snapshot = store.expect("the kind has a store").read();
    let text = |part: &Option<String>| part.clone().unwrap_or_default();
    let version = |object: &T| text(&object.meta().resource_version);
    snapshot
        .items
        .iter()
        .map(|(key, object)| ((key.namespace.to_string(), key.name.to_string()), version(object)))
        .collect()
}

fn of_full<K: Full>(stores: &ClusterStores) -> Versions {
    of_store(stores.full::<K>())
}

/// The objects of one kind, as the store of the app holds them now.
pub(crate) fn stored(stores: &ClusterStores, kind: Kind) -> Versions {
    match kind {
        Kind::Namespace => of_full::<Namespace>(stores),
        Kind::Node => of_full::<Node>(stores),
        Kind::Pod => of_full::<Pod>(stores),
        Kind::Deployment => of_full::<Deployment>(stores),
        Kind::StatefulSet => of_full::<StatefulSet>(stores),
        Kind::DaemonSet => of_full::<DaemonSet>(stores),
        Kind::ReplicaSet => of_full::<ReplicaSet>(stores),
        Kind::Job => of_full::<Job>(stores),
        Kind::CronJob => of_full::<CronJob>(stores),
        Kind::Service => of_full::<Service>(stores),
        Kind::EndpointSlice => of_full::<EndpointSlice>(stores),
        Kind::Event => of_full::<Event>(stores),
        metadata_only => of_store(stores.metadata(metadata_only)),
    }
}

/// The acceptance check after each fault: each store holds the objects of the API, each
/// one in its current version. The cluster changes by itself, for example its Events, so
/// the check reads again until the two sides agree, to the limit.
pub(crate) async fn assert_equal_to_the_api(stores: &ClusterStores, api: &Gateway, limit: Duration) {
    let end = Instant::now() + limit;
    let mut changes = stores.changes();
    let mut objects = 0;
    for kind in Kind::ALL.iter().copied().filter(|kind| kind.info().tier != Tier::C) {
        loop {
            let (in_api, in_store) = (versions(api, kind).await, stored(stores, kind));
            if in_api == in_store {
                objects += in_store.len();
                break;
            }
            let absent: Vec<_> = in_api.keys().filter(|key| !in_store.contains_key(*key)).collect();
            let extra: Vec<_> = in_store.keys().filter(|key| !in_api.contains_key(*key)).collect();
            let old: Vec<_> = in_api
                .iter()
                .filter(|(key, version)| in_store.get(*key).is_some_and(|stored| stored != *version))
                .collect();
            assert!(
                Instant::now() < end,
                "the store of {kind:?} is not equal to the API after {limit:?}: not in the store {absent:?}, \
                 only in the store {extra:?}, with an old version {old:?}"
            );
            tokio::select! {
                () = changes.next() => {}
                () = sleep(Duration::from_millis(200)) => {}
            }
        }
    }
    println!("each of the 39 stores is equal to the API: {objects} objects");
}

/// The ConfigMaps of the namespace `default`, which a test makes and changes.
pub(crate) struct ConfigMaps<'a> {
    api: &'a Gateway,
    request: Request,
}

impl<'a> ConfigMaps<'a> {
    pub(crate) fn of(api: &'a Gateway) -> Self {
        Self { api, request: Request::new("/api/v1/namespaces/default/configmaps") }
    }

    pub(crate) async fn create(&self, name: &str) {
        let body = json!({ "metadata": { "name": name, "labels": { "made-by": "fresnel-test" } }, "data": { "key": "value" } });
        let create = self.request.create(&PostParams::default(), serde_json::to_vec(&body).expect("JSON"));
        let _: Value =
            self.api.send(create.expect("a request")).await.unwrap_or_else(|error| panic!("create {name}: {error}"));
    }

    /// Makes the ConfigMaps `<prefix>-000` and so on, some at the same time.
    pub(crate) async fn create_many(&self, prefix: &str, count: usize) {
        let names: Vec<String> = (0..count).map(|number| format!("{prefix}-{number:03}")).collect();
        for group in names.chunks(20) {
            futures_util::future::join_all(group.iter().map(|name| self.create(name))).await;
        }
    }

    pub(crate) async fn label(&self, name: &str, value: &str) {
        let patch = Patch::Merge(json!({ "metadata": { "labels": { "round": value } } }));
        let request = self.request.patch(name, &PatchParams::default(), &patch).expect("a request");
        let _: Value = self.api.send(request).await.unwrap_or_else(|error| panic!("label {name}: {error}"));
    }

    pub(crate) async fn delete(&self, name: &str) {
        let request = self.request.delete(name, &DeleteParams::default()).expect("a request");
        let _: Value = self.api.send(request).await.unwrap_or_else(|error| panic!("delete {name}: {error}"));
    }
}

/// Makes a Secret as `kubectl apply` does: with a copy of its values in an annotation.
pub(crate) async fn create_applied_secret(api: &Gateway, name: &str, value: &str) {
    let copy = json!({ "data": { "password": value } }).to_string();
    let annotations = json!({ "kubectl.kubernetes.io/last-applied-configuration": copy });
    let body = json!({ "metadata": { "name": name, "annotations": annotations }, "data": { "password": value } });
    let request = Request::new("/api/v1/namespaces/default/secrets");
    let create = request.create(&PostParams::default(), serde_json::to_vec(&body).expect("JSON")).expect("a request");
    let _: Value = api.send(create).await.expect("the secret");
}
