//! The history of one cluster: each series with its samples, the drop of the series of an
//! object that left its store, and the memory limit (plan section 12.4).

use std::collections::BTreeMap;
use std::mem::size_of;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use super::series::{Sample, Series, Tier, Tiers};
use crate::kinds::Kind;
use crate::store::ObjectKey;

/// What one series is for. A key holds no cluster: each cluster has its own history, and
/// [`super::Metrics`] asks for the cluster in each lookup.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SeriesKey {
    /// One container of a pod.
    Container {
        /// The pod.
        pod: ObjectKey,
        /// The name of the container.
        name: Arc<str>,
    },
    /// One pod: the sum of its containers.
    Pod(ObjectKey),
    /// One workload: the sum of the pods that it owned at each poll, so the history stays
    /// over a rollout.
    Workload(Kind, ObjectKey),
    /// One node, with the usage of its system processes too.
    Node(Arc<str>),
    /// One namespace: the sum of its pods.
    Namespace(Arc<str>),
    /// The cluster: the sum of its nodes.
    Cluster,
}

impl SeriesKey {
    /// The place of the series in the sequence of the memory limit: the series of a
    /// container lose their samples first, the one of the cluster last.
    fn rank(&self) -> usize {
        match self {
            Self::Container { .. } => 0,
            Self::Pod(_) => 1,
            Self::Workload(..) => 2,
            Self::Namespace(_) => 3,
            Self::Node(_) => 4,
            Self::Cluster => 5,
        }
    }
}

const RANKS: usize = 6;
/// The part that one series costs besides its tiers: the series, its key, the node of the
/// map and the names of the key, two times: the sampler keeps one map of the series and
/// publishes a copy of it. An estimate: the limit is a setting, not a measure.
pub(crate) const SERIES_BYTES: usize = 2 * (size_of::<Series>() + size_of::<SeriesKey>() + 96);
/// The most that a sample of the last answer can be older than the last sample of its
/// series and still be of now, in seconds: the same sample again, or the one of a second
/// replica of the server, which scrapes on its own within the longest period of 60 s
/// (research 11 section 4.2).
const REPLICA_LAG: u32 = 60;
/// The time of the samples that one step of the memory limit drops from one rank.
const LIMIT_STEP: u32 = 60;

/// The series of one cluster.
#[derive(Clone, Debug, Default)]
pub(crate) struct History {
    series: BTreeMap<SeriesKey, Series>,
    /// The count of samples of all series.
    samples: usize,
    /// The bytes of the tiers of all series (`Series::bytes`).
    tiers: usize,
    /// The count of samples that the memory limit dropped (design rule 12).
    dropped: u64,
}

impl History {
    pub(crate) fn get(&self, key: &SeriesKey) -> Option<&Series> {
        self.series.get(key)
    }

    /// The series of each container of a pod, in the sequence of their names.
    pub(crate) fn containers<'a>(&'a self, pod: &'a ObjectKey) -> impl Iterator<Item = (&'a str, &'a Series)> {
        let first = SeriesKey::Container { pod: pod.clone(), name: "".into() };
        let of_pod =
            move |(key, _): &(&SeriesKey, &Series)| matches!(key, SeriesKey::Container { pod: of, .. } if of == pod);
        let named = |(key, series): (&'a SeriesKey, &'a Series)| match key {
            SeriesKey::Container { name, .. } => Some((&**name, series)),
            _ => None,
        };
        self.series.range(first..).take_while(of_pod).filter_map(named)
    }

    /// The count of series.
    pub(crate) fn len(&self) -> usize {
        self.series.len()
    }

    /// The memory of the history, as the limit counts it: the chunks of the tiers, which
    /// share no part with an other series, and each series in the two maps.
    pub(crate) fn bytes(&self) -> usize {
        self.tiers + self.series.len() * SERIES_BYTES
    }

    /// A copy that shares no part of a series, for the measure of the memory account.
    pub(crate) fn rebuilt(&self) -> Self {
        let series = self.series.iter().map(|(key, series)| (key.clone(), series.rebuilt())).collect();
        Self { series, ..*self }
    }

    /// The count of samples that the memory limit dropped.
    pub(crate) fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Adds the sample of a series of the answer `answer`. A sample that is not later than
    /// the last one of its series adds nothing, and marks the series as seen in that answer
    /// only when it is at most [`REPLICA_LAG`] older. A sample much older than the last one
    /// tells that the last one is not of now, for example of a clock that was ahead: the
    /// value of the series is then old (plan section 12.4).
    pub(crate) fn record(&mut self, key: SeriesKey, sample: Sample, answer: u64, tiers: &Tiers) {
        let series = self.series.entry(key).or_default();
        let before = (series.len(), series.bytes());
        series.push(sample, tiers);
        if series.latest().is_some_and(|latest| latest.at.saturating_sub(sample.at) <= REPLICA_LAG) {
            series.seen = answer;
        }
        self.samples = self.samples - before.0 + series.len();
        self.tiers = self.tiers - before.1 + series.bytes();
    }

    /// Marks the series of each object that left its store, and drops a series after `keep`
    /// (plan section 12.4: the drop of a deleted pod after one hour). `exists` tells if the
    /// object of a key is in its store: `None` while the store has no complete list, which
    /// changes no mark.
    pub(crate) fn sweep(&mut self, exists: impl Fn(&SeriesKey) -> Option<bool>, now: SystemTime, keep: Duration) {
        let (mut dropped, mut freed) = (0, 0);
        self.series.retain(|key, series| {
            match exists(key) {
                Some(true) => series.gone = None,
                Some(false) => series.gone = series.gone.or(Some(now)),
                None => {}
            }
            // A clock that went back gives no time since the mark.
            let stays = series.gone.is_none_or(|gone| now.duration_since(gone).unwrap_or_default() < keep);
            if !stays {
                dropped += series.len();
                freed += series.bytes();
            }
            stays
        });
        self.samples -= dropped;
        self.tiers -= freed;
    }

    /// Drops the oldest samples until the history fits in `max_bytes` (research 11 section
    /// 4.3): first the coarse tiers, rank by rank from the containers to the cluster, then
    /// the fine tiers in the same sequence. One step drops the samples of the oldest minute
    /// of one tier of one rank.
    pub(crate) fn limit(&mut self, max_bytes: usize) {
        for tier in [Tier::Coarse, Tier::Fine] {
            for rank in 0..RANKS {
                while self.bytes() > max_bytes {
                    let in_rank = |key: &SeriesKey| key.rank() == rank;
                    let ranked = self.series.iter().filter(|(key, _)| in_rank(key));
                    let Some(oldest) = ranked.filter_map(|(_, series)| series.oldest(tier)).min() else {
                        break;
                    };
                    let last = oldest.saturating_add(LIMIT_STEP - 1);
                    for (_, series) in self.series.iter_mut().filter(|(key, _)| in_rank(key)) {
                        let bytes = series.bytes();
                        let freed = series.drop_through(tier, last);
                        self.samples -= freed;
                        self.dropped += freed as u64;
                        self.tiers = self.tiers - bytes + series.bytes();
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
