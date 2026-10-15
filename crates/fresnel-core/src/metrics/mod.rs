//! The metrics of the clusters (plan section 12.4): the CPU and the memory of containers,
//! pods, workloads, nodes, namespaces and clusters, with a history in memory.
//!
//! One [`Sampler`] for each connected cluster polls `metrics.k8s.io`, a poll store with a
//! state for each of its two lists. A reader gets [`Metrics`] through the published state
//! type: each poll store, each series and the requests on each node.
//!
//! - `sampler`: the task. `state`: the state of a poll store. `view`: what a reader holds.
//! - `wire`: the answers of the server. `quantity` and `duration`: own readers of their texts.
//! - `world`: what a poll reads of the stores. `rollup`: the sums of a poll.
//! - `series`: one series in two tiers. `history`: the series of a cluster and their limits.
//! - `resources`: the requests and the limits of a pod.

mod duration;
mod history;
mod quantity;
mod resources;
mod rollup;
mod series;
mod wire;
mod world;

pub use history::SeriesKey;
pub use resources::{Amount, Limits, PodResources, of_pod};
pub use series::{Cover, Sample, Series};
pub use world::{Inputs, NodeLoad};
