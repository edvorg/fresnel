//! What the CPU and memory columns of a table read of one cluster: the last value of each
//! object with its request and its limit, or with the allocatable part of a node.
//!
//! The worker thread of a table reads it for each row, so a sort compares the values of
//! one poll. The usage of a workload is the sum of its pods at each poll in the core; its
//! request and its limit are the sums of its pods that are not done, at the time of the
//! rows (research 11 section 6). A match always uses the cluster and the name.

use std::any::Any;

use fresnel_core::app::ClusterData;
use fresnel_core::cluster::ClusterId;
use fresnel_core::index::{Index, Owners};
use fresnel_core::kinds::{Describe, Kind, Object};
use fresnel_core::metrics::{self, Amount, Latest, Limits, Metrics, PodResources, SeriesKey};
use fresnel_core::published::Held;
use fresnel_core::store::{ObjectKey, Snapshot};
use k8s_openapi::api::core::v1::{Node, Pod};

use super::gauge::{Gauge, Resource, Usage, Whole};
use crate::views::objects::uid;

/// The metrics of one cluster with the objects that a sum of requests reads.
#[derive(Clone)]
pub(crate) struct Metered {
    pub(crate) cluster: ClusterId,
    pub(super) metrics: Held<Metrics>,
    pub(super) owners: Held<Index<Owners>>,
    pods: Held<Snapshot<Pod>>,
}

impl Metered {
    pub(crate) fn of(cluster: &ClusterId, data: &ClusterData) -> Self {
        let (metrics, owners, pods) = (data.metrics.clone(), data.owners.clone(), data.pods.clone());
        Self { cluster: cluster.clone(), metrics, owners, pods }
    }

    /// True when the two hold the same poll of the same cluster: the rows need no new values.
    pub(crate) fn is_same(&self, other: &Self) -> bool {
        self.cluster == other.cluster && std::ptr::eq::<Metrics>(&*self.metrics, &*other.metrics)
    }

    /// The usage of an object of a kind with the columns. `None` for an object with no
    /// sample, which is not a value of zero (plan section 12.4).
    pub(crate) fn usage<K: Describe>(&self, key: &ObjectKey, object: &K) -> Option<Usage> {
        let any: &dyn Any = object;
        if let Some(pod) = any.downcast_ref::<Pod>() {
            return self.pod(key, pod);
        }
        if let Some(node) = any.downcast_ref::<Node>() {
            return self.node(&key.name, node);
        }
        super::has_columns(K::ID).then(|| self.workload(K::ID, key, uid(object.meta())))?
    }

    fn latest(&self, key: SeriesKey) -> Option<Latest> {
        self.metrics.latest(&self.cluster, &key)
    }

    fn pod(&self, key: &ObjectKey, pod: &Pod) -> Option<Usage> {
        let latest = self.latest(SeriesKey::Pod(key.clone()))?;
        let PodResources { requests, limits } = metrics::of_pod(pod);
        let usage = latest.sample.usage;
        Some(Usage {
            cpu: gauge(usage.cpu, requests.cpu, limits.cpu),
            memory: gauge(usage.memory, requests.memory, limits.memory),
            ..of_latest(latest, Whole::Limit)
        })
    }

    /// A workload: the sum of the pods at each poll, against the requests and the limits of
    /// its pods that are not done. One pod with no limit gives the sum no limit.
    fn workload(&self, kind: Kind, key: &ObjectKey, uid: &str) -> Option<Usage> {
        let latest = self.latest(SeriesKey::Workload(kind, key.clone()))?;
        let owned = self.owners.pods_of(kind, uid).found;
        let pods = owned.iter().filter_map(|pod| self.pods.items.get(&pod.key));
        let running: Vec<PodResources> =
            pods.filter(|pod| !metrics::is_done(pod)).map(|pod| metrics::of_pod(pod)).collect();
        let requests: Amount = running.iter().map(|resources| resources.requests).sum();
        let limit = |read: fn(&Limits) -> Option<u64>| {
            let mut limits = running.iter().map(|resources| read(&resources.limits));
            let first = limits.next()??;
            limits.try_fold(first, |sum, limit| Some(sum.saturating_add(limit?)))
        };
        let usage = latest.sample.usage;
        Some(Usage {
            cpu: gauge(usage.cpu, requests.cpu, limit(|limits| limits.cpu)),
            memory: gauge(usage.memory, requests.memory, limit(|limits| limits.memory)),
            ..of_latest(latest, Whole::Limit)
        })
    }

    /// A node: its usage with its system processes, against the requests of its pods at
    /// the last poll and its allocatable part.
    fn node(&self, name: &str, node: &Node) -> Option<Usage> {
        let latest = self.latest(SeriesKey::Node(name.into()))?;
        let requests = self.metrics.node_load(&self.cluster, name).map(|load| load.requests);
        let allocatable = node.status.as_ref().and_then(|status| status.allocatable.as_ref());
        let (usage, requests) = (latest.sample.usage, requests.unwrap_or_default());
        Some(Usage {
            cpu: gauge(usage.cpu, requests.cpu, Resource::Cpu.read(allocatable)),
            memory: gauge(usage.memory, requests.memory, Resource::Memory.read(allocatable)),
            ..of_latest(latest, Whole::Allocatable)
        })
    }
}

/// One resource. A request of zero is no request: the API gives a pod with no request the
/// value zero.
fn gauge(usage: u64, request: u64, most: Option<u64>) -> Gauge {
    Gauge { usage, request: (request > 0).then_some(request), most }
}

/// The parts of a usage that the sample gives, with no amount yet.
fn of_latest(latest: Latest, whole: Whole) -> Usage {
    let (cpu, memory) = (Gauge::default(), Gauge::default());
    Usage { cpu, memory, whole, at: latest.sample.time(), cover: latest.sample.cover, current: latest.current }
}
