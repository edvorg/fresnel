//! The metrics of the showcase: 30 minutes of polls, one each 15 s, as a sampler of the core
//! gives them. Each pod has a value of its app and a wave of its own, the shop apps grow
//! with the morning traffic, and each sum is the sum of its parts at the same poll.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::SystemTime;

use fresnel_core::cluster::ClusterId;
use fresnel_core::kinds::Kind;
use fresnel_core::metrics::{self, Amount, Cover, Metrics, MetricsSettings, NodeLoad, Sample, SeriesKey};
use fresnel_core::store::ObjectKey;

use super::objects::{App, Owner, State, hash};
use super::world::Planned;
use crate::metrics::{PERIOD, POLLS, poll_at, poll_time};

/// A MiB as a factor of a value of the world.
const MI: f64 = crate::metrics::MI as f64;
/// The usage of the system processes of a node: millicores and MiB.
const SYSTEM: [f64; 2] = [160.0, 720.0];

/// The sum of each series at one poll, with the count of its parts.
type Sums = BTreeMap<SeriesKey, (Amount, u16)>;

/// A value near 1 that moves in waves of the object of `seed`, with some noise.
fn wave(seed: u64, poll: u32, depth: f64) -> f64 {
    let (time, phase) = (f64::from(poll), (seed % 628) as f64 / 100.0);
    let noise = (hash(&[&seed.to_string(), &poll.to_string()]) % 1000) as f64 / 1000.0 - 0.5;
    1.0 + depth * (0.6 * (time / 11.0 + phase).sin() + 0.25 * (time / 3.1 + 2.0 * phase).sin() + 0.3 * noise)
}

/// The usage of one pod of an app at a poll.
fn usage(app: &App, pod: &str, poll: u32) -> Amount {
    let seed = hash(&[pod]);
    let own = 0.8 + (seed % 400) as f64 / 1000.0;
    // The shop gets more visits from 07:30 on.
    let traffic = if app.namespace == "shop" { 0.7 + 0.45 * f64::from(poll) / f64::from(POLLS) } else { 1.0 };
    let [millicores, mib] = app.usage;
    let cpu = millicores * own * traffic * wave(seed, poll, 0.35);
    // The heap grows until a collection, each 10 to 13 minutes.
    let cycle = 40 + seed % 12;
    let heap = 0.92 + 0.15 * ((u64::from(poll) + seed) % cycle) as f64 / cycle as f64;
    let memory = mib * (0.9 + 0.2 * own) * heap * wave(seed / 7, poll, 0.02);
    Amount { cpu: (cpu * 1e6) as u64, memory: (memory * MI) as u64 }
}

/// The workloads of a pod whose sums the core keeps, from its app and its controller.
fn workloads(app: &App, owner: Option<&str>) -> Vec<SeriesKey> {
    let key = |kind, name: &str| SeriesKey::Workload(kind, ObjectKey::new(app.namespace, name));
    match app.owner {
        Owner::Deployment(..) => {
            let replica_set = owner.map(|name| key(Kind::ReplicaSet, name));
            [key(Kind::Deployment, app.name)].into_iter().chain(replica_set).collect()
        }
        Owner::StatefulSet => vec![key(Kind::StatefulSet, app.name)],
        Owner::DaemonSet => vec![key(Kind::DaemonSet, app.name)],
        Owner::CronJob => Vec::new(),
    }
}

/// Adds a part to the sum of a series.
fn add(sums: &mut Sums, key: SeriesKey, usage: Amount) {
    let sum = sums.entry(key).or_default();
    *sum = (sum.0 + usage, sum.1 + 1);
}

/// The sums of one poll: each container, pod, workload, namespace and node, and the
/// cluster. A pod counts from its creation on, while its container runs: metrics-server has
/// no sample of a container that waits, as one in CrashLoopBackOff does.
fn sums(cluster: &str, pods: &[Planned], poll: u32) -> Sums {
    let mut sums = Sums::new();
    for Planned { pod, app, state } in pods {
        let node = pod.spec.as_ref().and_then(|spec| spec.node_name.as_deref());
        let born = pod.metadata.creation_timestamp.as_ref().map(|time| SystemTime::from(time.0));
        let runs = matches!(state, State::Running(..));
        let (Some(node), true, false) = (node, runs, born.is_some_and(|born| born > poll_time(poll))) else {
            continue;
        };
        let name = pod.metadata.name.as_deref().unwrap_or_default();
        let used = usage(app, name, poll);
        let key = ObjectKey::new(app.namespace, name);
        add(&mut sums, SeriesKey::Container { pod: key.clone(), name: app.name.into() }, used);
        add(&mut sums, SeriesKey::Pod(key), used);
        let owner = pod.metadata.owner_references.iter().flatten().next().map(|owner| owner.name.as_str());
        for workload in workloads(app, owner) {
            add(&mut sums, workload, used);
        }
        add(&mut sums, SeriesKey::Namespace(app.namespace.into()), used);
        add(&mut sums, SeriesKey::Node(node.into()), used);
    }
    // A node has the usage of its system processes too; the cluster is the sum of its nodes.
    let nodes: Vec<Arc<str>> = sums
        .keys()
        .filter_map(|key| if let SeriesKey::Node(name) = key { Some(Arc::clone(name)) } else { None })
        .collect();
    for name in nodes {
        let seed = hash(&[cluster, &name]);
        let cpu = (SYSTEM[0] * wave(seed, poll, 0.2) * 1e6) as u64;
        let memory = ((SYSTEM[1] + (seed % 64) as f64) * MI) as u64;
        let key = SeriesKey::Node(name);
        add(&mut sums, key.clone(), Amount { cpu, memory });
        let node = sums.get(&key).map(|sum| sum.0).unwrap_or_default();
        add(&mut sums, SeriesKey::Cluster, node);
    }
    sums
}

/// The requests on each node: the pods that are not done.
fn loads(pods: &[Planned]) -> BTreeMap<Arc<str>, NodeLoad> {
    let mut loads: BTreeMap<Arc<str>, NodeLoad> = BTreeMap::new();
    for Planned { pod, .. } in pods.iter().filter(|planned| !metrics::is_done(&planned.pod)) {
        if let Some(node) = pod.spec.as_ref().and_then(|spec| spec.node_name.as_deref()) {
            let load = loads.entry(node.into()).or_default();
            (load.requests, load.pods) = (load.requests + metrics::of_pod(pod).requests, load.pods + 1);
        }
    }
    loads
}

/// The metrics of a cluster with these pods. `last` is the time of its last answer: the
/// polls of a cluster that lost its connection end there.
pub(super) fn metrics(cluster: &str, pods: &[Planned], last: SystemTime) -> Metrics {
    let mut metrics = Metrics::waiting(ClusterId::new(cluster), PERIOD, poll_time(0) - PERIOD);
    for poll in (0..POLLS).filter(|poll| poll_time(*poll) <= last) {
        let at = poll_at(poll);
        let samples: Vec<(SeriesKey, Sample)> = sums(cluster, pods, poll)
            .into_iter()
            .map(|(key, (usage, count))| {
                let cover = match key {
                    SeriesKey::Workload(..) => Cover { sampled: count, of: count },
                    _ => Cover::ONE,
                };
                (key, Sample { at, cover, usage })
            })
            .collect();
        metrics.take_samples(&samples, loads(pods), poll_time(poll), &MetricsSettings::default());
    }
    metrics
}
