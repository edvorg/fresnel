//! The clusters of the showcase as the core gives them to the views: the stores of each
//! kind, the indexes, the metrics, the permissions, the connections and the journal.

use std::collections::BTreeMap;
use std::sync::Arc;

use fresnel_core::CoreError;
use fresnel_core::app::{AccessView, ClusterData, ClusterRow, CoreView, Objects, RulesState};
use fresnel_core::cluster::{ServerVersion, VersionState};
use fresnel_core::index::{Index, IndexState, Owners, References};
use fresnel_core::journal::{JournalView, Level};
use fresnel_core::kinds::{Describe, Kind, Meta, Tier};
use fresnel_core::kubeconfig::AuthKind;
use fresnel_core::metrics::{Metrics, PollState};
use fresnel_core::net::{ChannelState, ChannelStatus, ClusterStatus, ErrorClass};
use fresnel_core::ops::access::Rules;
use fresnel_core::search::Source;
use fresnel_core::store::{Snapshot, StoreState, Wait};
use k8s_openapi::api::core::v1::Event;
use serde_json::json;

use super::stores::Stores;
use super::usage;
use super::world::{NAMESPACES, PROD_EU, PROD_US, STAGING, Site};
use crate::data::{event_index, held, store};
use crate::fake::{SECOND, channel, connected, journal_of, live_status, now, status, valid, view};

/// The seconds since prod-us lost its connection: its stores have no data since then.
pub(super) const LOST: u32 = 48;
/// Why prod-us is not live, as the core tells it: the purpose and the last error of its first
/// channel that is not live.
pub(super) const PROD_US_REASON: &str = "watches: connect: no answer in 3s";
/// The seconds since the last answer of the metrics API of prod-us: it polls each 15 s, and
/// the poll after that answer came after the loss.
const LAST_ANSWER: u32 = LOST + 7;
/// The seconds since the first connect of prod-us after the loss failed.
const RETRY: u32 = LOST - 8;
/// The seconds since the start of the session.
const START: u32 = 2460;

/// The snapshot of a store of a cluster: live, or waiting for a connection.
fn snapshot<T: fresnel_core::kinds::Object>(site: &Site, objects: &[Arc<T>]) -> Snapshot<T> {
    let state = if site.live { StoreState::Live } else { StoreState::Waiting(Wait::Channel) };
    store(state, objects)
}

/// The rules of a user who can do each verb on each resource of each namespace.
fn access() -> Arc<AccessView> {
    let rules = NAMESPACES.map(|namespace| {
        let review = serde_json::from_value(json!({
            "spec": {"namespace": namespace},
            "status": {"resourceRules": [{"verbs": ["*"], "apiGroups": ["*"], "resources": ["*"]}],
                "nonResourceRules": [], "incomplete": false},
        }))
        .expect("a review");
        (Arc::from(namespace), RulesState::Read(Arc::new(Rules::of(namespace, review).expect("rules"))))
    });
    Arc::new(AccessView { rules: BTreeMap::from(rules), checks: BTreeMap::new() })
}

/// The stores that the core holds of a cluster: for each kind a source of the quick search
/// and of the counts of the sidebar, and the full objects for the views.
#[derive(Default)]
struct Holder {
    objects: Objects,
    sources: Vec<Source>,
}

impl Holder {
    /// A store of a kind of tier B has the metadata only; a view gets its full objects.
    fn hold<K: Describe>(&mut self, site: &Site, items: &[Arc<K>]) {
        let full = held(snapshot(site, items));
        self.sources.push(match K::TIER {
            Tier::B => Source::metadata(K::ID, held(snapshot(site, &crate::search::metadata(items)))),
            Tier::A | Tier::C => Source::full(full.clone()),
        });
        self.objects = std::mem::take(&mut self.objects).with(full);
    }
}

/// The metrics of a cluster. The polls of a cluster that lost its connection end before
/// the loss, and its lists of the pods and of the nodes fail since the first retry.
fn metrics(site: &Site, stores: &Stores) -> Metrics {
    if site.live {
        return usage::metrics(site.name, &stores.pods, now());
    }
    let history = usage::metrics(site.name, &stores.pods, now() - LAST_ANSWER * SECOND);
    let error = CoreError::new(ErrorClass::Network, "no connection to the cluster");
    let mut metrics = crate::metrics::failed(history, PollState::Error(error), now() - RETRY * SECOND);
    (metrics.nodes.state, metrics.nodes.since) = (metrics.pods.state.clone(), metrics.pods.since);
    metrics
}

/// The data of a cluster: each store, the indexes, the metrics and the version.
fn data(site: &Site, stores: &Stores) -> ClusterData {
    let pods = held(snapshot(site, &stores.pods.iter().map(|planned| Arc::clone(&planned.pod)).collect::<Vec<_>>()));
    let (replica_sets, jobs) = (snapshot(site, &stores.replica_sets), snapshot(site, &stores.jobs));
    let slices = snapshot(site, &stores.endpoint_slices);
    let owners = Index::<Owners>::of_snapshots(&replica_sets, &jobs, &pods, &slices, IndexState::Following);
    let mut holder = Holder { sources: vec![Source::full(pods.clone())], ..Holder::default() };
    holder.hold(site, &stores.deployments);
    holder.hold(site, &stores.replica_sets);
    holder.hold(site, &stores.stateful_sets);
    holder.hold(site, &stores.daemon_sets);
    holder.hold(site, &stores.budgets);
    holder.hold(site, &stores.jobs);
    holder.hold(site, &stores.cron_jobs);
    holder.hold(site, &stores.services);
    holder.hold(site, &stores.endpoint_slices);
    holder.hold(site, &stores.nodes);
    holder.hold(site, &stores.namespaces);
    holder.hold(site, &stores.events);
    holder.hold(site, &stores.config_maps);
    holder.hold(site, &stores.secrets);
    // The store of each other kind of tier A or B has no object, as in a cluster with none.
    for kind in Kind::ALL.iter().copied().filter(|kind| kind.info().tier != Tier::C) {
        if !holder.sources.iter().any(|source| source.status.kind == kind) {
            holder.sources.push(Source::metadata(kind, held(snapshot::<Meta>(site, &[]))));
        }
    }
    let version =
        ServerVersion { version: Some(site.version.into()), state: VersionState::Read(now() - 3600 * SECOND) };
    ClusterData {
        events: Some(event_index(snapshot::<Event>(site, &stores.events), IndexState::Following)),
        owners: held(owners),
        references: held(Index::<References>::of_pods(&pods, IndexState::Following)),
        stores: holder.sources.iter().map(|source| source.status.clone()).collect(),
        pods,
        sources: holder.sources.into(),
        objects: holder.objects,
        keys: None,
        access: access(),
        metrics: held(metrics(site, stores)),
        version: held(version),
        logs: Arc::from([]),
        saves: Arc::from([]),
    }
}

/// The connections of a cluster that lost them: each channel tries again after a pause.
fn lost() -> ClusterStatus {
    let lost = |failures, pause: u32| ChannelStatus {
        since: now() - SECOND,
        last_frame_age: Some(LOST * SECOND),
        connect_attempts: u64::from(failures),
        last_error: Some("connect: no answer in 3s".to_owned()),
        ..channel(ChannelState::Backoff { failures, pause: pause * SECOND })
    };
    status([lost(6, 4), lost(6, 7), lost(5, 9)], Some(valid()))
}

/// The row of a cluster with its data.
fn cluster(site: &Site) -> ClusterRow {
    let row = connected(site.name, if site.live { live_status() } else { lost() });
    let auth = Some(AuthKind::Plugin("kubelogin".to_owned()));
    ClusterRow { auth, data: Some(data(site, &Stores::of(site))), ..row }
}

/// The journal of the session: the start 41 minutes ago, and the loss of prod-us.
fn journal() -> Arc<JournalView> {
    let live = "live: a request got an answer on each channel";
    let lost = "a connection was lost (watches: the server sent no ping answer in 5s); it connects again";
    let retry = "no connection (watches: connect: no answer in 3s); the channel tries again";
    let entries = [
        (START, Level::Info, "app", "read 3 contexts from 1 kubeconfig file"),
        (START - 1, Level::Info, "prod-eu", live),
        (START - 1, Level::Info, "staging", live),
        (START - 2, Level::Info, "prod-us", live),
        (LOST, Level::Warning, "prod-us", lost),
        (RETRY, Level::Warning, "prod-us", retry),
    ];
    journal_of(&entries.map(|(seconds, level, source, text)| (seconds * SECOND, level, source, text)), 0)
}

/// The view of the core with the three clusters and the journal.
pub(super) fn world() -> CoreView {
    let mut core = view(vec![cluster(&PROD_EU), cluster(&PROD_US), cluster(&STAGING)]);
    core.journal = journal();
    core
}
