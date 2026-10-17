//! One time series in two tiers (plan section 12.4): each sample of the last hour, and one
//! sample for each minute of the last 24 hours (research 11 section 4.3).
//!
//! A tier is a vector of `imbl`: a copy shares each part that did not change, so the sampler
//! publishes the full history after each poll and copies only the new end of each series.

use std::time::{Duration, SystemTime};

use imbl::Vector;

use super::resources::Amount;

/// How many objects of a sum had a value at the time of a sample: "3 of 4 pods have
/// samples" (research 11 section 6). A missing value is not zero, so a view tells it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cover {
    /// The objects with a value in the sum.
    pub sampled: u16,
    /// The objects that the sum is for. Never below `sampled`.
    pub of: u16,
}

impl Cover {
    /// The value of one object.
    pub const ONE: Self = Self { sampled: 1, of: 1 };

    /// `sampled` objects of `of`. A count above `u16::MAX` stays at it. A store that has
    /// no complete list yet can know fewer objects than the sum has, so `of` is at least
    /// `sampled`.
    pub(crate) fn new(sampled: usize, of: usize) -> Self {
        let count = |count: usize| u16::try_from(count).unwrap_or(u16::MAX);
        Self { sampled: count(sampled), of: count(of.max(sampled)) }
    }
}

/// One value of a series.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample {
    /// The time of the sample on the clock of the cluster, in seconds since 1970: the
    /// timestamp of the item of the server. For a sum, the latest time of its items.
    pub at: u32,
    /// The objects of a sum with a value.
    pub cover: Cover,
    /// The usage. In a sample of the coarse tier: the mean CPU and the largest memory of
    /// its minute.
    pub usage: Amount,
}

impl Sample {
    /// The time of the sample as a time of the system.
    pub fn time(&self) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(self.at.into())
    }
}

/// The samples of one chunk of a tier: the vector of `imbl` holds its elements in chunks of
/// 64, and a tier of 3 samples holds a full chunk.
const CHUNK: usize = 64;
/// The samples that a vector of `imbl` holds in its own struct, before its first chunk.
const INLINE: usize = 2;
/// The bytes of one chunk: its samples, its two ends and the counts of its shared pointer.
/// A measure of the counting allocator gives 1,568 bytes.
pub(crate) const CHUNK_BYTES: usize = CHUNK * size_of::<Sample>() + 4 * size_of::<usize>();

/// The bytes of a tier of this many samples on the heap, as a measure of the vectors of
/// `imbl` gives them, and some more: one chunk up to 64 samples; then a chunk for each 64,
/// two more at the ends of a tier that drops at its front and adds at its back, and one for
/// the node of the middle, four chunks at least.
fn tier_bytes(samples: usize) -> usize {
    let chunks = if samples <= INLINE {
        0
    } else if samples <= CHUNK {
        1
    } else {
        (samples.div_ceil(CHUNK) + 2).max(4) + 1
    };
    chunks * CHUNK_BYTES
}

/// One tier of a series.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tier {
    /// Each sample.
    Fine,
    /// One sample for each step.
    Coarse,
}

/// The lengths of the tiers, in seconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Tiers {
    /// The time that the fine tier covers.
    pub(crate) fine: u32,
    /// The time of one sample of the coarse tier.
    pub(crate) step: u32,
    /// The time that the coarse tier covers.
    pub(crate) coarse: u32,
}

/// The samples of one step of the coarse tier, before the step ends.
#[derive(Clone, Copy, Debug)]
struct Step {
    start: u32,
    last: u32,
    cpu: u128,
    count: u32,
    memory: u64,
    cover: Cover,
}

impl Step {
    fn of(start: u32, sample: Sample) -> Self {
        let Amount { cpu, memory } = sample.usage;
        Self { start, last: sample.at, cpu: cpu.into(), count: 1, memory, cover: sample.cover }
    }

    fn add(&mut self, sample: Sample) {
        self.last = sample.at;
        self.cpu += u128::from(sample.usage.cpu);
        self.count += 1;
        self.memory = self.memory.max(sample.usage.memory);
        self.cover =
            Cover { sampled: self.cover.sampled.min(sample.cover.sampled), of: self.cover.of.max(sample.cover.of) };
    }

    /// The sample of the step: at the time of its last sample, with the mean CPU and the
    /// largest memory. The cover is the smallest part of the step.
    fn sample(&self) -> Sample {
        let cpu = u64::try_from(self.cpu / u128::from(self.count.max(1))).unwrap_or(u64::MAX);
        Sample { at: self.last, cover: self.cover, usage: Amount { cpu, memory: self.memory } }
    }
}

/// The samples of one container, pod, workload, node, namespace or cluster.
#[derive(Clone, Debug, Default)]
pub struct Series {
    fine: Vector<Sample>,
    coarse: Vector<Sample>,
    step: Option<Step>,
    /// The time of one step of the coarse tier, in seconds, from the last push.
    step_length: u32,
    latest: Option<Sample>,
    /// The number of the last answer of the server with a value of this series that is not
    /// much older than `latest` (see `History::record`).
    pub(crate) seen: u64,
    /// The time of the system when the object of the series left its store.
    pub(crate) gone: Option<SystemTime>,
}

impl Series {
    /// The last sample.
    pub fn latest(&self) -> Option<Sample> {
        self.latest
    }

    /// The samples in the sequence of their time: the steps of the coarse tier before the
    /// first sample of the fine tier, then the fine tier.
    pub fn samples(&self) -> impl Iterator<Item = Sample> + '_ {
        let first_fine = self.fine_from();
        let older = move |sample: &&Sample| first_fine.is_none_or(|first| sample.at < first);
        self.coarse.iter().filter(older).chain(self.fine.iter()).copied()
    }

    /// The distance of two samples of the coarse tier, in seconds: one step. A view expects
    /// one sample each step before [`Series::fine_from`] and one each period after it, and
    /// shows a gap where samples are missing.
    pub fn step(&self) -> u32 {
        self.step_length
    }

    /// The time of the first sample of the fine tier. Each sample before it is of the
    /// coarse tier.
    pub fn fine_from(&self) -> Option<u32> {
        self.fine.front().map(|sample| sample.at)
    }

    /// The count of samples that the tiers hold.
    pub(crate) fn len(&self) -> usize {
        self.fine.len() + self.coarse.len()
    }

    /// The bytes of the two tiers on the heap.
    pub(crate) fn bytes(&self) -> usize {
        tier_bytes(self.fine.len()) + tier_bytes(self.coarse.len())
    }

    /// Adds a sample that is later than the last one, and drops each sample that is older
    /// than its tier. False for a sample that is not later: the same sample in a second
    /// answer, or an older one of a second replica of the server (research 11 section 4.2).
    pub(crate) fn push(&mut self, sample: Sample, tiers: &Tiers) -> bool {
        if self.latest.is_some_and(|latest| sample.at <= latest.at) {
            return false;
        }
        self.latest = Some(sample);
        self.step_length = tiers.step;
        self.fine.push_back(sample);
        let start = sample.at - sample.at % tiers.step.max(1);
        match &mut self.step {
            Some(step) if step.start == start => step.add(sample),
            open => {
                if let Some(ended) = open.replace(Step::of(start, sample)) {
                    self.coarse.push_back(ended.sample());
                }
            }
        }
        self.drop_through(Tier::Fine, sample.at.saturating_sub(tiers.fine));
        self.drop_through(Tier::Coarse, sample.at.saturating_sub(tiers.coarse));
        true
    }

    /// The time of the oldest sample of a tier.
    pub(crate) fn oldest(&self, tier: Tier) -> Option<u32> {
        self.tier(tier).front().map(|sample| sample.at)
    }

    /// Drops each sample of a tier at `last` or before. Gives their count. The bound is
    /// inclusive, so a drop through the time of the oldest sample always drops it, also at
    /// the last second of `u32`.
    pub(crate) fn drop_through(&mut self, tier: Tier, last: u32) -> usize {
        let samples = match tier {
            Tier::Fine => &mut self.fine,
            Tier::Coarse => &mut self.coarse,
        };
        let before = samples.len();
        while samples.front().is_some_and(|sample| sample.at <= last) {
            samples.pop_front();
        }
        // A vector of `imbl` keeps its chunks when its samples go. A tier with fewer chunks
        // by `tier_bytes` gets a new vector, so the limit counts what the tier holds: a soak
        // measured the series of deleted pods with each sample gone and a full chunk each.
        if tier_bytes(samples.len()) < tier_bytes(before) {
            *samples = samples.iter().copied().collect();
        }
        before - samples.len()
    }

    /// A copy that shares no part of the tiers, which a push of each sample builds: the
    /// memory account measures the bytes of the series with it.
    pub(crate) fn rebuilt(&self) -> Self {
        Self { fine: self.fine.iter().copied().collect(), coarse: self.coarse.iter().copied().collect(), ..*self }
    }

    fn tier(&self, tier: Tier) -> &Vector<Sample> {
        match tier {
            Tier::Fine => &self.fine,
            Tier::Coarse => &self.coarse,
        }
    }
}

#[cfg(test)]
mod tests;
