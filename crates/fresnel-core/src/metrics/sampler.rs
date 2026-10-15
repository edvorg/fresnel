//! The sampler of one cluster: a task that reads the PodMetrics and the NodeMetrics of all
//! namespaces each period, and publishes the metrics after each poll.
//!
//! The sampler runs while its cluster is connected, also when no view shows metrics: the
//! history is then there when a view opens (research 11 section 4.1). Each list is one
//! read of the gateway, with its limit of 10 s and no retry. A failed poll changes the
//! state of its list, and the next poll comes at the next period.

use std::convert::Infallible;
use std::panic::{AssertUnwindSafe, resume_unwind};
use std::sync::Arc;

use fresnel_net::WallClock;
use fresnel_net::task::{Backoff, Restart, TaskHandle, TaskName, Tasks, panic_message};
use futures_util::FutureExt as _;
use futures_util::future::join;
use kube::core::Request;
use kube::core::params::ListParams;
use tokio::time::{Instant, sleep_until};

use super::MetricsSettings;
use super::state::PollState;
use super::view::Metrics;
use super::wire::{self, RawList};
use super::world::Inputs;
use crate::cluster::{ClusterId, Gateway};
use crate::error::CoreError;
use crate::published::{Bin, Notice, Publisher, Reader};

/// The reads of the lists. The gateway is the one source of the app; a unit test gives
/// its answers with a script.
pub(crate) trait Lists: Send + Sync + 'static {
    /// Reads the list of one path of `metrics.k8s.io`.
    fn list(&self, path: &'static str) -> impl Future<Output = Result<RawList, CoreError>> + Send;
}

impl Lists for Gateway {
    async fn list(&self, path: &'static str) -> Result<RawList, CoreError> {
        let request = Request::new(path).list(&ListParams::default());
        self.get(request.map_err(|error| CoreError::invalid(format!("the list of {path}: {error}")))?).await
    }
}

/// What the owner of a sampler gives to it.
#[derive(Clone)]
pub struct SamplerParts {
    /// The settings.
    pub settings: MetricsSettings,
    /// Frees the values that the readers held.
    pub bin: Bin,
    /// Hears each publish, for the repaint.
    pub notice: Notice,
    /// The clock of the times that a state shows.
    pub clock: WallClock,
}

/// What each run of the task reads.
struct Run<S> {
    lists: S,
    inputs: Inputs,
    publisher: Publisher<Metrics>,
    settings: MetricsSettings,
    clock: WallClock,
}

/// The sampler of one cluster with its task. The task stops when this value drops, and the
/// readers then see the state "stopped" with the last values.
pub struct Sampler {
    publisher: Publisher<Metrics>,
    clock: WallClock,
    _task: TaskHandle,
}

impl Sampler {
    /// Starts the sampler of the cluster of `gateway`. `inputs` reads the stores and the
    /// owner index of the same cluster.
    pub fn start(tasks: &Tasks, gateway: Gateway, inputs: Inputs, parts: SamplerParts) -> Self {
        let cluster = gateway.cluster().clone();
        Self::with_lists(tasks, cluster, gateway, inputs, parts)
    }

    pub(crate) fn with_lists<S: Lists>(
        tasks: &Tasks,
        cluster: ClusterId,
        lists: S,
        inputs: Inputs,
        parts: SamplerParts,
    ) -> Self {
        let SamplerParts { settings, bin, notice, clock } = parts;
        let first = Metrics::waiting(cluster.clone(), settings.period, clock.now());
        let publisher = Publisher::new(first, bin, notice);
        let run = Arc::new(Run { lists, inputs, publisher: publisher.clone(), settings, clock: clock.clone() });
        let name = TaskName::scoped("metrics sampler", cluster.as_str());
        let task = tasks.start(name, Restart::Always(Backoff::default()), move || self::run(Arc::clone(&run)));
        Self { publisher, clock, _task: task }
    }

    /// A reader of the metrics.
    pub fn reader(&self) -> Reader<Metrics> {
        self.publisher.reader()
    }
}

impl Drop for Sampler {
    fn drop(&mut self) {
        let now = self.clock.now();
        self.publisher.close(|last| last.in_state(&PollState::Stopped, now));
    }
}

/// One run of the task. A run ends only with a panic: the metrics then show the state
/// "internal error", and the start function gives the report and starts a new run.
async fn run<S: Lists>(run: Arc<Run<S>>) -> Result<(), Infallible> {
    let panic = match AssertUnwindSafe(follow(&run)).catch_unwind().await {
        Ok(never) => match never {},
        Err(panic) => panic,
    };
    let state = PollState::InternalError(format!("the task panicked: {}", panic_message(panic.as_ref())).into());
    let failed = run.publisher.read().in_state(&state, run.clock.now());
    run.publisher.publish(failed);
    resume_unwind(panic)
}

/// Polls with no end. The two reads go at once: they share a channel of HTTP/2. A run after
/// a fault goes on with the history of the last value: one bad poll must not cost the
/// history of a day.
async fn follow<S: Lists>(run: &Run<S>) -> Infallible {
    let mut metrics = Metrics::clone(&run.publisher.read());
    loop {
        let start = Instant::now();
        let (pods, nodes) = join(run.lists.list(wire::PODS), run.lists.list(wire::NODES)).await;
        let world = run.inputs.read();
        metrics.take(pods.map(wire::pods), nodes.map(wire::nodes), world, run.clock.now(), &run.settings);
        run.publisher.publish(metrics.clone());
        sleep_until(start + metrics.period).await;
    }
}

#[cfg(test)]
mod tests;
