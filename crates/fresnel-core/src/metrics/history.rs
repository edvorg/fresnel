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
/// The part of the memory limit that one sample costs.
const SAMPLE_BYTES: usize = size_of::<Sample>();
/// The part that one series costs besides its samples: the series, its key, the node of the
/// map and the names of the key. An estimate: the limit is a setting, not a measure.
const SERIES_BYTES: usize = size_of::<Series>() + size_of::<SeriesKey>() + 96;
/// The time of the samples that one step of the memory limit drops from one rank.
const LIMIT_STEP: u32 = 60;

/// The series of one cluster.
#[derive(Clone, Debug, Default)]
pub(crate) struct History {
    series: BTreeMap<SeriesKey, Series>,
    /// The count of samples of all series.
    samples: usize,
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

    /// The memory of the history, as the limit counts it.
    pub(crate) fn bytes(&self) -> usize {
        self.samples * SAMPLE_BYTES + self.series.len() * SERIES_BYTES
    }

    /// The count of samples that the memory limit dropped.
    pub(crate) fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Adds the sample of a series of the answer `answer`. A sample that is not later than
    /// the last one of its series only marks the series as seen in that answer.
    pub(crate) fn record(&mut self, key: SeriesKey, sample: Sample, answer: u64, tiers: &Tiers) {
        let series = self.series.entry(key).or_default();
        let before = series.len();
        series.seen = answer;
        series.push(sample, tiers);
        self.samples = self.samples - before + series.len();
    }

    /// Marks the series of each object that left its store, and drops a series after `keep`
    /// (plan section 12.4: the drop of a deleted pod after one hour). `exists` tells if the
    /// object of a key is in its store: `None` while the store has no complete list, which
    /// changes no mark.
    pub(crate) fn sweep(&mut self, exists: impl Fn(&SeriesKey) -> Option<bool>, now: SystemTime, keep: Duration) {
        let mut dropped = 0;
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
            }
            stays
        });
        self.samples -= dropped;
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
                    let ranked = self.series.iter_mut().filter(|(key, _)| in_rank(key));
                    let freed: usize = ranked.map(|(_, series)| series.drop_through(tier, last)).sum();
                    self.samples -= freed;
                    self.dropped += freed as u64;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
