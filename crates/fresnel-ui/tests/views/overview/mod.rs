//! The overviews of the Cluster and the Workloads groups on the fake core (milestone M7): a
//! block for each selected cluster, the counts of each workload kind by health, their links
//! to the tables, and the marks of a cluster whose stores or sampler are not live.
//!
//! - `cluster`, `workloads`: the tests of each overview. `snapshots`: PNG files.
//! - `warnings`: the warnings of the Cluster overview with the state of each problem now.
//!   `quiet`: the reason of each state on its row, and a warning with no report for long.
//! - `edges`: a stream of snapshots, and texts of a cluster with no limit of their length.
//! - `frame_time`: four clusters with 10,000 pods each. Its limit holds in a release build.

mod cluster;
mod edges;
mod frame_time;
mod quiet;
mod snapshots;
mod warnings;
mod workloads;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::SystemTime;

use egui::accesskit::Role;
use egui_kittest::kittest::Queryable as _;
use fresnel_core::app::{ClusterRow, Objects};
use fresnel_core::cluster::{ClusterId, ServerVersion, VersionState};
use fresnel_core::kinds::Describe;
use fresnel_core::metrics::{Amount, Cover, Metrics, MetricsSettings, NodeLoad, Sample, SeriesKey};
use fresnel_core::search::Source;
use fresnel_core::store::StoreState;
use k8s_openapi::api::apps::v1::{DaemonSet, Deployment, ReplicaSet, StatefulSet};
use k8s_openapi::api::batch::v1::{CronJob, Job};
use k8s_openapi::api::core::v1::{Namespace, Node, ReplicationController};

use crate::data::{events, held, live, pods, scene, store};
use crate::fake::{SECOND, Window, now, open_page, until};

/// The node of the scene: each cluster of these tests has a node of this name, with an
/// allocatable part of 18 cores.
const NODE: &str = "35f62a67b541";
const GI: u64 = 1024 * 1024 * 1024;

/// The metrics of a cluster after one poll 5 s ago: its node and the sum of its nodes, and
/// the requests on the node. `scale` gives a second cluster values of its own.
fn metrics(cluster: &str, scale: u64) -> Metrics {
    let mut metrics = Metrics::waiting(ClusterId::new(cluster), 15 * SECOND, now() - 20 * SECOND);
    let at = now().duration_since(SystemTime::UNIX_EPOCH).expect("a time").as_secs() - 5;
    let usage = Amount { cpu: 1_500_000_000 * scale, memory: 6 * GI * scale };
    let sample = Sample { at: u32::try_from(at).expect("a time of u32"), cover: Cover::ONE, usage };
    let requests = Amount { cpu: 2_500_000_000 * scale, memory: 4 * GI * scale };
    let loads = BTreeMap::from([(Arc::from(NODE), NodeLoad { requests, pods: 20 })]);
    let samples = [(SeriesKey::Node(NODE.into()), sample), (SeriesKey::Cluster, sample)];
    metrics.take_samples(&samples, loads, now(), &MetricsSettings::default());
    metrics
}

/// The objects of a kind, as the core holds them for an overview (`Command::ShowKinds`).
fn with<K: Describe>(objects: Objects, items: &[Arc<K>]) -> Objects {
    objects.with(held(store(StoreState::Live, items)))
}

/// The store of the full objects of a kind that is always on.
fn always<K: Describe>(items: &[Arc<K>]) -> Source {
    Source::full(held(store(StoreState::Live, items)))
}

/// A live cluster of the scene with each kind of the two overviews, the metrics of
/// `scale`, and the version of its server. It has no ReplicationController. Its store of
/// the Namespaces gives the menu of the namespace filter its entries, and its stores of the
/// nodes and the workloads tell the state of each warning now.
pub(crate) fn cluster(name: &str, scale: u64, version: &str) -> ClusterRow {
    let row = live(name, &pods());
    let mut data = row.data.clone().expect("the data of a connected cluster");
    let objects = with(with(data.objects, &scene::<Node>()), &events());
    let objects = with(with(with(objects, &scene::<Deployment>()), &scene::<StatefulSet>()), &scene::<DaemonSet>());
    let objects = with(with(with(objects, &scene::<ReplicaSet>()), &scene::<Job>()), &scene::<CronJob>());
    data.objects = with(objects, &Vec::<Arc<ReplicationController>>::new());
    data.metrics = held(metrics(name, scale));
    data.sources = Arc::from([
        always(&scene::<Namespace>()),
        always(&scene::<Node>()),
        always(&scene::<ReplicaSet>()),
        always(&scene::<Job>()),
        always(&scene::<CronJob>()),
    ]);
    let state = VersionState::Read(now() - 60 * SECOND);
    data.version = held(ServerVersion { version: Some(version.into()), state });
    ClusterRow { data: Some(data), ..row }
}

/// Opens an overview and waits for the counts of the worker.
pub(crate) fn open(window: &mut Window, page: &str) {
    open_page(window, page);
    until(window, "the counts", |window| {
        window.query_by_label("counting…").is_none() && window.query_all_by_role(Role::Link).next().is_some()
    });
}
