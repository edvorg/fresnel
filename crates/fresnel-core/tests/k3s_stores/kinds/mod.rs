//! Each kind of the app on a real cluster: its store, its columns, and its state when the
//! server refuses it or does not serve it.
//!
//! - `scene`: an object of each kind that k3s does not have by itself.
//! - `tables`: the tables of the server, the comparison with the columns of the app, and
//!   their copy for the unit tests. `reader`: a user who cannot read one kind.

mod reader;
mod scene;
mod tables;

use std::pin::Pin;
use std::time::Duration;

use fresnel_core::kinds::{Describe, Kind, Object, Tier, Visit};
use fresnel_core::published::Reader;
use fresnel_core::store::{ClusterStores, Items, Snapshot, StoreSettings, StoreState, StoreStatus};
use k8s_openapi::api::autoscaling::v2::HorizontalPodAutoscaler;
use k8s_openapi::api::coordination::v1::Lease;
use k8s_openapi::api::core::v1::ConfigMap;
use tokio::time::{Instant, sleep, timeout};

use crate::api::{Versions, versions};
use crate::bed::{App, Cluster, LIMIT};
use crate::store_bed::Stores;

/// The pause between two reads of a state that gives no signal to the test.
const STEP: Duration = Duration::from_millis(200);

/// The steps are in one test, because the start of a cluster is the slow part.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs k3s in Docker"]
async fn each_kind_lists_with_the_columns_of_the_server_and_a_forbidden_kind_leaves_the_others_live() {
    let cluster = Cluster::start("kinds").await;
    let reader = reader::Reader::create(&cluster).await;
    let took = scene::create(&cluster).await;
    println!("the scene is at rest after {took:?}");

    let app = App::start(&cluster, StoreSettings::default()).await;
    app.stores.until_live(LIMIT).await;
    each_kind_lists_its_objects(&app.stores.stores, &cluster).await;
    tables::each_column_has_the_text_of_the_server(&cluster).await;
    tables::write_if_asked(&cluster).await;

    let as_reader = reader.app(&cluster);
    one_kind_is_forbidden_and_each_other_one_is_live(&as_reader.stores).await;
    drop(as_reader);
    assert_eq!(app.proxy.task_failures(), Vec::<String>::new());
}

/// Plan section 9.7 on a real server: a kind that the cluster does not serve, as a cluster
/// before Kubernetes 1.23 has no `autoscaling/v2`. The API server of k3s does not get ready
/// with `node.k8s.io` off: its own informers read the RuntimeClasses.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs k3s in Docker"]
async fn a_kind_that_the_server_does_not_serve_is_not_supported_and_each_other_one_is_live() {
    let cluster = Cluster::start_without("no-autoscaling-v2", "autoscaling/v2").await;
    let app = App::start(&cluster, StoreSettings::default()).await;
    let statuses = until_all_but(&app.stores, Kind::HorizontalPodAutoscaler).await;
    let refused = statuses.iter().find(|status| status.kind == Kind::HorizontalPodAutoscaler).expect("its store");
    let text = "not supported: the server could not find the requested resource (HTTP 404)";
    assert_eq!(refused.state.to_string(), text);

    let demand = app.stores.stores.demand::<HorizontalPodAutoscaler>().expect("a store on demand");
    let shown =
        until_state(&app.stores.stores, demand.reader(), |state| matches!(state, StoreState::NotSupported { .. }));
    assert_eq!(shown.await.to_string(), text);
    let leases = app.stores.stores.demand::<Lease>().expect("a store on demand");
    until_state(&app.stores.stores, leases.reader(), StoreState::is_live).await;
    assert_eq!(app.proxy.task_failures(), Vec::<String>::new());
}

/// Holds a store of each kind and waits until each one is live and has each object of
/// the API, in its current version.
async fn each_kind_lists_its_objects(stores: &ClusterStores, cluster: &Cluster) {
    let mut objects = 0;
    for kind in Kind::ALL.iter().copied() {
        let listed = kind.visit(Listed { stores, cluster }).await;
        assert!(listed > 0, "the scene has an object of the kind {kind:?}");
        objects += listed;
    }
    println!("each of the {} kinds has a live store, with {objects} objects", Kind::ALL.len());
}

struct Listed<'a> {
    stores: &'a ClusterStores,
    cluster: &'a Cluster,
}

impl<'a> Visit for Listed<'a> {
    type Output = Pin<Box<dyn Future<Output = usize> + Send + 'a>>;

    /// A tier A kind gives its store that is always on, and each other kind a store of
    /// its full objects on demand.
    fn visit<K: Describe>(self) -> Self::Output {
        Box::pin(async move {
            let demand = self.stores.demand::<K>().expect("a store of the kind");
            assert_eq!(self.stores.full::<K>().is_some(), K::TIER == Tier::A, "{:?}", K::ID);
            until_state(self.stores, demand.reader(), StoreState::is_live).await;
            let api = &self.cluster.api.gateway;
            let end = Instant::now() + LIMIT;
            loop {
                let (in_api, in_store) = (versions(api, K::ID).await, stored(&demand.read().items));
                if in_api == in_store {
                    return in_store.len();
                }
                assert!(Instant::now() < end, "the store of {:?} is not equal to the API: {in_store:?}", K::ID);
                // The cluster changes by itself, for example its Events and its Leases.
                sleep(STEP).await;
            }
        })
    }
}

fn stored<K: Describe>(items: &Items<K>) -> Versions {
    let text = |part: &Option<String>| part.clone().unwrap_or_default();
    items
        .iter()
        .map(|(key, object)| ((key.namespace.to_string(), key.name.to_string()), text(&object.meta().resource_version)))
        .collect()
}

/// Plan section 9.7: HTTP 403 for one kind shows "forbidden" for its store, and each other
/// store is live.
async fn one_kind_is_forbidden_and_each_other_one_is_live(stores: &Stores) {
    let statuses = until_all_but(stores, Kind::ConfigMap).await;
    let state = &statuses.iter().find(|status| status.kind == Kind::ConfigMap).expect("its store").state;
    let text = "forbidden: configmaps is forbidden: User \"fresnel-reader\" cannot list resource \"configmaps\" in \
                API group \"\" at the cluster scope (HTTP 403)";
    assert_eq!(state.to_string(), text);
    let demand = stores.stores.demand::<ConfigMap>().expect("a store on demand");
    let shown = until_state(&stores.stores, demand.reader(), |state| matches!(state, StoreState::Forbidden { .. }));
    assert_eq!(shown.await.to_string(), text);
    let leases = stores.stores.demand::<Lease>().expect("a store on demand");
    until_state(&stores.stores, leases.reader(), StoreState::is_live).await;
    println!("{} stores are live, and the store of the ConfigMaps shows: {text}", statuses.len() - 1);
}

/// Waits until each store is live but the one of `refused`, which shows a refusal.
async fn until_all_but(stores: &Stores, refused: Kind) -> Vec<StoreStatus> {
    let settled = |all: &[StoreStatus]| {
        all.iter().all(|status| match status.kind == refused {
            true => matches!(status.state, StoreState::Forbidden { .. } | StoreState::NotSupported { .. }),
            false => status.state.is_live(),
        })
    };
    stores.until("each store is live but one", LIMIT, settled).await
}

/// Waits until a store shows a state that meets the condition, and gives that state.
async fn until_state<T: Send + Sync + 'static>(
    stores: &ClusterStores,
    reader: &Reader<Snapshot<T>>,
    met: impl Fn(&StoreState) -> bool,
) -> StoreState {
    let mut changes = stores.changes();
    let wait = async {
        loop {
            let state = reader.read().state.clone();
            if met(&state) {
                return state;
            }
            changes.next().await;
        }
    };
    timeout(LIMIT, wait).await.unwrap_or_else(|_| panic!("in time: the state {:?}", reader.read().state))
}
