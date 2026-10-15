//! The metrics of the clusters (plan section 12.4): the CPU and the memory of containers,
//! pods, workloads, nodes, namespaces and clusters, with a history in memory.
//!
//! One [`Sampler`] for each connected cluster polls `metrics.k8s.io`, a poll store with a
//! state for each of its two lists. A reader gets [`Metrics`] through the published state
//! type: each poll store, each series and the requests on each node.
//!
//! - `sampler`: the task. `state`: the state of a poll store. `view`: what a reader holds.
//! - `wire`: the answers of the server. `quantity` and `duration`: own readers of their texts;
//!   the view of a ResourceQuota reads its quantities with `quantity::number` too.
//! - `world`: what a poll reads of the stores. `rollup`: the sums of a poll.
//! - `series`: one series in two tiers. `history`: the series of a cluster and their limits.
//! - `resources`: the requests and the limits of a pod.

mod duration;
mod history;
pub mod quantity;
mod resources;
mod rollup;
mod sampler;
mod series;
mod state;
mod view;
mod wire;
mod world;

use std::time::Duration;

pub use history::SeriesKey;
pub use resources::{Amount, Limits, PodResources, is_done, of_pod};
pub use sampler::{Sampler, SamplerParts};
pub use series::{Cover, Sample, Series};
pub use state::{PollState, PollStatus};
pub use view::{Latest, Metrics};
pub use world::{Inputs, NodeLoad};

/// The settings of the sampler of each cluster.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetricsSettings {
    /// The time between two polls: the resolution that the manifest and the Helm chart of
    /// metrics-server set. A shorter one gives the same values again.
    pub period: Duration,
    /// The time that the fine tier of a series covers, with each sample.
    pub fine: Duration,
    /// The time of one sample of the coarse tier.
    pub step: Duration,
    /// The time that the coarse tier of a series covers.
    pub coarse: Duration,
    /// The time that the series of an object stay after it left its store.
    pub keep_gone: Duration,
    /// The most bytes of the history of one cluster. A limit for each cluster: a large
    /// cluster then cannot push the history of a small one out.
    pub memory_limit: usize,
}

impl Default for MetricsSettings {
    /// A series of 24 hours has 1,680 samples of 24 bytes, about 40 KB. 64 MiB then hold
    /// about 1,600 full series: the containers, pods and workloads of about 500 pods.
    fn default() -> Self {
        Self {
            period: Duration::from_secs(15),
            fine: Duration::from_secs(3_600),
            step: Duration::from_secs(60),
            coarse: Duration::from_secs(24 * 3_600),
            keep_gone: Duration::from_secs(3_600),
            memory_limit: 64 * 1024 * 1024,
        }
    }
}
