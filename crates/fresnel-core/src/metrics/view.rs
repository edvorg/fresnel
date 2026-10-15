//! The metrics of one cluster as a reader holds them, and the step of one poll.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use super::history::{History, SeriesKey};
use super::series::{Sample, Series, Tiers};
use super::service::Availability;
use super::state::{ABSENT_PAUSE, PollState, PollStatus};
use super::wire::{NodeSample, PodSample, Read};
use super::world::{NodeLoad, World};
use super::{MetricsSettings, rollup};
use crate::cluster::ClusterId;
use crate::error::CoreError;
use crate::store::ObjectKey;

/// The longest time between two polls that the window of the server sets. metrics-server
/// has 60 s as its default resolution (research 11 section 4.2).
const LONGEST_PERIOD: Duration = Duration::from_secs(60);

/// The answers of one poll.
pub(crate) struct Poll {
    pub(crate) pods: Result<Read<PodSample>, CoreError>,
    pub(crate) nodes: Result<Read<NodeSample>, CoreError>,
    /// The read of the APIService after a 503 of a list.
    pub(crate) service: Option<Availability>,
}

/// The last value of a series, and if the last answer had it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Latest {
    /// The last sample.
    pub sample: Sample,
    /// True when the last poll of its list got the list, and the list had the object at
    /// the time of this sample or a little before it (a second replica of the server). A
    /// value that is not current shows as old (plan section 12.4).
    pub current: bool,
}

/// The metrics of one cluster at one moment. Each lookup takes the cluster, and a lookup
/// with an other cluster finds nothing: two clusters can have a node of the same name
/// (weakness 2 of plan section 6).
#[derive(Clone, Debug)]
pub struct Metrics {
    /// The cluster of the metrics.
    pub cluster: ClusterId,
    /// The poll store of the PodMetrics: the containers, pods, workloads and namespaces.
    pub pods: PollStatus,
    /// The poll store of the NodeMetrics: the nodes and the cluster.
    pub nodes: PollStatus,
    /// The time between two polls: the setting, or the window of the server when it is
    /// longer (research 11 section 4.2).
    pub period: Duration,
    /// The time of the system of the next poll. `None` before the first poll.
    pub next: Option<SystemTime>,
    history: History,
    loads: Arc<BTreeMap<Arc<str>, NodeLoad>>,
}

impl Metrics {
    /// The metrics before the first poll.
    pub fn waiting(cluster: ClusterId, period: Duration, now: SystemTime) -> Self {
        let (pods, nodes) = (PollStatus::new(now), PollStatus::new(now));
        Self { cluster, pods, nodes, period, next: None, history: History::default(), loads: Arc::default() }
    }

    /// The series of an object of `cluster`.
    pub fn series(&self, cluster: &ClusterId, key: &SeriesKey) -> Option<&Series> {
        self.of(cluster)?.history.get(key)
    }

    /// The last value of an object of `cluster`, with its freshness.
    pub fn latest(&self, cluster: &ClusterId, key: &SeriesKey) -> Option<Latest> {
        let series = self.series(cluster, key)?;
        let status = match key {
            SeriesKey::Node(_) | SeriesKey::Cluster => &self.nodes,
            _ => &self.pods,
        };
        let current = status.state.is_ok() && series.seen == status.answers;
        Some(Latest { sample: series.latest()?, current })
    }

    /// The series of each container of a pod of `cluster`, in the sequence of their names.
    pub fn containers<'a>(
        &'a self,
        cluster: &ClusterId,
        pod: &'a ObjectKey,
    ) -> impl Iterator<Item = (&'a str, &'a Series)> {
        self.of(cluster).into_iter().flat_map(move |metrics| metrics.history.containers(pod))
    }

    /// The requests on a node of `cluster` at the last poll. `None` for a node with no pod
    /// that is not done.
    pub fn node_load(&self, cluster: &ClusterId, node: &str) -> Option<NodeLoad> {
        self.of(cluster)?.loads.get(node).copied()
    }

    /// The count of series.
    pub fn series_count(&self) -> usize {
        self.history.len()
    }

    /// The memory of the history, as the limit of the settings counts it.
    pub fn bytes(&self) -> usize {
        self.history.bytes()
    }

    /// The count of samples that the memory limit dropped.
    pub fn dropped(&self) -> u64 {
        self.history.dropped()
    }

    fn of(&self, cluster: &ClusterId) -> Option<&Self> {
        (self.cluster == *cluster).then_some(self)
    }

    /// The time from a poll to the next one: the period, or a longer pause for a cluster
    /// with no `metrics.k8s.io`.
    pub fn pause(&self) -> Duration {
        let absent = self.pods.state.is_absent() && self.nodes.state.is_absent();
        if absent { ABSENT_PAUSE.max(self.period) } else { self.period }
    }

    /// The same metrics in a new state of both poll stores: a fault or the stop of the task.
    pub(crate) fn in_state(&self, state: &PollState, now: SystemTime) -> Self {
        let mut metrics = self.clone();
        metrics.pods.set(state.clone(), now);
        metrics.nodes.set(state.clone(), now);
        metrics.next = None;
        metrics
    }

    /// Takes the answers of one poll and the stores of that moment: the samples of each
    /// series, the drop of the series of the objects that are gone, the memory limit and
    /// the requests of the nodes.
    pub(crate) fn take(&mut self, poll: Poll, world: World, now: SystemTime, settings: &MetricsSettings) {
        let tiers = settings.tiers();
        let Poll { pods, nodes, service } = poll;
        self.pods.take(pods.as_ref().map_err(Clone::clone), service.as_ref(), now);
        self.nodes.take(nodes.as_ref().map_err(Clone::clone), service.as_ref(), now);
        let mut windows = Vec::new();
        if let Ok(read) = &nodes {
            windows.extend(read.items.iter().filter_map(|node| node.window));
            for (key, sample) in rollup::of_nodes(&read.items, &world) {
                self.history.record(key, sample, self.nodes.answers, &tiers);
            }
        }
        if let Ok(read) = &pods {
            if windows.is_empty() {
                windows.extend(read.items.iter().filter_map(|pod| pod.window));
            }
            for (key, sample) in rollup::of_pods(&read.items, &world) {
                self.history.record(key, sample, self.pods.answers, &tiers);
            }
        }
        self.period = period(settings.period, windows);
        self.history.sweep(|key| world.exists(key), now, settings.keep_gone);
        self.history.limit(settings.memory_limit);
        self.loads = Arc::new(world.loads);
        self.next = Some(now + self.pause());
    }

    /// Takes one answer of each list with these samples of the sums and the requests on
    /// each node, as a poll gives them. A test of a reader builds the metrics of each state
    /// with it, with no server. A series with no sample in the answer has an old value.
    pub fn take_samples(
        &mut self,
        samples: &[(SeriesKey, Sample)],
        loads: BTreeMap<Arc<str>, NodeLoad>,
        now: SystemTime,
        settings: &MetricsSettings,
    ) {
        let of_nodes = |key: &SeriesKey| matches!(key, SeriesKey::Node(_) | SeriesKey::Cluster);
        let nodes = samples.iter().filter(|(key, _)| of_nodes(key)).count();
        self.nodes.answer(nodes, now);
        self.pods.answer(samples.len() - nodes, now);
        for (key, sample) in samples {
            let answer = if of_nodes(key) { self.nodes.answers } else { self.pods.answers };
            self.history.record(key.clone(), *sample, answer, &settings.tiers());
        }
        self.loads = Arc::new(loads);
        self.next = Some(now + self.period);
    }
}

/// The period of the setting, or the median window of the last answer when it is longer:
/// a poll before the next window of the server gives the same values again.
fn period(setting: Duration, mut windows: Vec<Duration>) -> Duration {
    windows.sort_unstable();
    let median = windows.get(windows.len() / 2).copied().unwrap_or_default();
    setting.max(median.min(LONGEST_PERIOD))
}

impl MetricsSettings {
    /// The tiers of each series, in seconds.
    fn tiers(&self) -> Tiers {
        let seconds = |duration: Duration| u32::try_from(duration.as_secs()).unwrap_or(u32::MAX);
        Tiers { fine: seconds(self.fine), step: seconds(self.step), coarse: seconds(self.coarse) }
    }
}

#[cfg(test)]
mod tests;
