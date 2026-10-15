//! The metrics on the fake core (plan section 12.4): the CPU and memory columns of the
//! tables, the Metrics tab of a panel, and the sampler of each cluster in the Connections
//! view. The metrics of each cluster come from `Metrics::take_samples`, as a sampler gives
//! them after each poll.
//!
//! - `tables`: the columns, an old value, a missing value and the weaknesses 2 and 4.
//! - `tab`: the charts, their tooltip, their gaps and the range. `sampler`: the Connections
//!   view. `snapshots`: PNG files.

mod sampler;
mod snapshots;
mod tab;
mod tables;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use fresnel_core::app::ClusterRow;
use fresnel_core::cluster::ClusterId;
use fresnel_core::kinds::{Describe, Kind};
use fresnel_core::metrics::{self, Amount, Cover, Metrics, MetricsSettings, NodeLoad, PollState, Sample, SeriesKey};
use fresnel_core::store::{ObjectKey, StoreState};
use k8s_openapi::api::core::v1::Pod;
use serde_json::json;

use crate::data::{changed, held, live, pods, store};
use crate::fake::{SECOND, now};

/// The node of the scene. Each cluster of these tests has a node of this name.
pub(crate) const NODE: &str = "35f62a67b541";
/// The polls of a history: 30 minutes, one each 15 s, the last one 5 s before now.
const POLLS: u32 = 120;
const PERIOD: Duration = Duration::from_secs(15);
/// The polls in which the pod `crash` had no sample: a restart of 2 minutes, 10 minutes ago.
const RESTART: std::ops::Range<u32> = 80..88;
/// The pod `leaving` has no sample in the last two answers: its value is old.
const LEAVING: &str = "leaving";
const MI: u64 = 1024 * 1024;

/// The pods of the scene, with requests and limits on the pods of `web` and a request with
/// no limit on the pods of `db`.
pub(crate) fn scene_pods() -> Vec<Arc<Pod>> {
    let resources = |name: &str| match name {
        _ if name.starts_with("web-") => Some(json!({
            "requests": {"cpu": "100m", "memory": "64Mi"},
            "limits": {"cpu": "500m", "memory": "128Mi"},
        })),
        "db-0" | "db-1" => Some(json!({"requests": {"cpu": "250m", "memory": "256Mi"}})),
        _ => None,
    };
    let with = |pod: Arc<Pod>| match resources(pod.metadata.name.as_deref().unwrap_or_default()) {
        Some(resources) => changed(&pod, |value| value["spec"]["containers"][0]["resources"] = resources),
        None => pod,
    };
    pods().into_iter().map(with).collect()
}

/// The time of a poll on the clock of the cluster.
fn poll_time(poll: u32) -> SystemTime {
    now() - 5 * SECOND - PERIOD * (POLLS - 1 - poll)
}

/// The usage of one container at one poll: a wave around a value of the container, which
/// it has at the last poll. `scale` makes the values of a second cluster other values: two
/// clusters with the same scene have values of their own.
pub(crate) fn container(pod: &str, name: &str, poll: u32, scale: u64) -> Amount {
    let wave = (f64::from(poll) - f64::from(POLLS - 1)) / 6.0;
    let (millicores, mib) = match pod {
        // Near its limit of 500m and 128Mi.
        "web-6888b778b6-75md7" => (460.0, 120.0),
        _ => {
            let seed: u64 = pod.bytes().chain(name.bytes()).map(u64::from).sum();
            (20.0 + (seed % 80) as f64, 24.0 + (seed % 40) as f64)
        }
    };
    let cpu = millicores * (1.0 + 0.25 * wave.sin()) * 1e6;
    let memory = mib * (1.0 + 0.1 * wave.sin()) * MI as f64;
    Amount { cpu: cpu as u64 * scale, memory: memory as u64 * scale }
}

/// The metrics of a cluster with the pods of `scene_pods`: one poll each 15 s for 30
/// minutes. The workloads `web`, `db` and `agent` have the sums of their pods, the node the
/// sum of each pod and its system processes.
pub(crate) fn scene_metrics(cluster: &str, scale: u64) -> Metrics {
    let pods = scene_pods();
    let running: Vec<&Arc<Pod>> = pods.iter().filter(|pod| runs(pod)).collect();
    let mut loads = BTreeMap::new();
    let requests: Amount =
        pods.iter().filter(|pod| !metrics::is_done(pod)).map(|pod| metrics::of_pod(pod).requests).sum();
    loads.insert(Arc::from(NODE), NodeLoad { requests, pods: running.len() as u32 });
    let workloads = [
        (Kind::Deployment, "web", "web-"),
        (Kind::ReplicaSet, "web-6888b778b6", "web-"),
        (Kind::StatefulSet, "db", "db-"),
        (Kind::DaemonSet, "agent", "agent-"),
    ];
    let mut metrics = Metrics::waiting(ClusterId::new(cluster), PERIOD, poll_time(0) - PERIOD);
    for poll in 0..POLLS {
        let at = u32::try_from(poll_time(poll).duration_since(SystemTime::UNIX_EPOCH).expect("a time").as_secs())
            .expect("a time of u32");
        let sample = |usage: Amount, sampled, of| Sample { at, cover: Cover { sampled, of }, usage };
        let mut samples = Vec::new();
        let mut by_pod = BTreeMap::new();
        for pod in &running {
            let name = pod.metadata.name.as_deref().unwrap_or_default();
            if (name == "crash" && RESTART.contains(&poll)) || (name == LEAVING && poll + 2 >= POLLS) {
                continue;
            }
            let key = ObjectKey::new("scene", name);
            let containers = pod.spec.as_ref().map(|spec| spec.containers.clone()).unwrap_or_default();
            let mut sum = Amount::default();
            for each in &containers {
                let usage = container(name, &each.name, poll, scale);
                sum = sum + usage;
                let key = SeriesKey::Container { pod: key.clone(), name: each.name.as_str().into() };
                samples.push((key, sample(usage, 1, 1)));
            }
            samples.push((SeriesKey::Pod(key), sample(sum, 1, 1)));
            by_pod.insert(name.to_owned(), sum);
        }
        for (kind, name, prefix) in workloads {
            let owned: Vec<Amount> =
                by_pod.iter().filter(|(pod, _)| pod.starts_with(prefix)).map(|(_, usage)| *usage).collect();
            let count = u16::try_from(owned.len()).expect("a count");
            let key = SeriesKey::Workload(kind, ObjectKey::new("scene", name));
            samples.push((key, sample(owned.into_iter().sum(), count, count)));
        }
        let system = Amount { cpu: 150_000_000 * scale, memory: 512 * MI * scale };
        let node = by_pod.values().copied().sum::<Amount>() + system;
        samples.push((SeriesKey::Node(NODE.into()), sample(node, 1, 1)));
        metrics.take_samples(&samples, loads.clone(), poll_time(poll), &MetricsSettings::default());
    }
    metrics
}

/// True for a pod that runs on the node: it has a sample.
fn runs(pod: &Pod) -> bool {
    pod.status.as_ref().and_then(|status| status.phase.as_deref()) == Some("Running")
}

/// A live cluster with the pods of `scene_pods` and these metrics.
pub(crate) fn of_pods(name: &str, metrics: Metrics) -> ClusterRow {
    let row = live(name, &scene_pods());
    let mut data = row.data.clone().expect("the data of a connected cluster");
    data.metrics = held(metrics);
    ClusterRow { data: Some(data), ..row }
}

/// A live cluster with the pods of `scene_pods`, these objects of a kind and these metrics,
/// as the core gives them to a view that shows the kind.
pub(crate) fn of_kind<K: Describe>(name: &str, objects: &[Arc<K>], metrics: Metrics) -> ClusterRow {
    let row = of_pods(name, metrics);
    let mut data = row.data.clone().expect("the data of a connected cluster");
    data.objects = data.objects.with(held(store(StoreState::Live, objects)));
    ClusterRow { data: Some(data), ..row }
}

/// The metrics of a cluster whose list of the pods fails since `since`, with the history of
/// `metrics`.
pub(crate) fn failed(mut metrics: Metrics, state: PollState, since: SystemTime) -> Metrics {
    (metrics.pods.state, metrics.pods.since) = (state, since);
    metrics
}
