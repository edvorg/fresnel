//! The metrics in the views (plan section 12.4): the CPU and memory columns of the tables
//! of the Pods, the Nodes and the workloads, and the Metrics tab of their panels. The
//! sampler of each cluster in `fresnel-core` gives the values; a view never asks a server.
//!
//! A value that is old shows as old, with the reason, and a value that is absent shows as
//! absent, never as zero. Each value tells the age of its sample.
//!
//! - `gauge`: one value with its request and limit, and the rules of its bar.
//! - `read`: the value of each object of a cluster. `cells`: the cells of a table.
//! - `tab`: the Metrics tab. `lines`: the lines of its charts.

pub(crate) mod cells;
pub(crate) mod gauge;
mod lines;
pub(crate) mod read;
pub(crate) mod tab;

use fresnel_core::kinds::Kind;
use fresnel_core::metrics::{Metrics, PollStatus};

use crate::format::Clock;

/// True for a kind whose table has the CPU and memory columns and whose panel has the
/// Metrics tab: the Pods, the Nodes and the workloads.
pub(crate) const fn has_columns(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::Pod
            | Kind::Node
            | Kind::Deployment
            | Kind::StatefulSet
            | Kind::DaemonSet
            | Kind::ReplicaSet
            | Kind::ReplicationController
            | Kind::Job
            | Kind::CronJob
    )
}

/// The state of each list of the sampler of a cluster in words, when it is not ok: each
/// metric of the cluster then shows it (research 11 section 8.1).
#[derive(Clone, Debug, Default)]
pub(crate) struct Problems {
    /// The PodMetrics: the containers, the pods and the workloads.
    pods: Option<String>,
    /// The NodeMetrics: the nodes.
    nodes: Option<String>,
}

impl Problems {
    pub(crate) fn of(metrics: &Metrics, clock: &Clock) -> Self {
        let text = |status: &PollStatus, list: &str| {
            let since = clock.time_of_day(status.since);
            (!status.state.is_ok()).then(|| format!("{list}: {} (since {since})", status.state))
        };
        Self { pods: text(&metrics.pods, "PodMetrics"), nodes: text(&metrics.nodes, "NodeMetrics") }
    }

    /// The problem of the list that has the values of a kind.
    pub(crate) fn of_kind(&self, kind: Kind) -> Option<&str> {
        if kind == Kind::Node { self.nodes.as_deref() } else { self.pods.as_deref() }
    }
}
