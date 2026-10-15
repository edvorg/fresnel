//! One log view: the task that follows its pods, runs one source for each stream, merges
//! their lines into its ring and publishes the [`LogView`] (plan section 12.7).
//!
//! - `targets`: the streams that the view wants now. `run`: the task of the view.

mod run;
mod targets;

use std::sync::Arc;

use fresnel_net::task::{Backoff, Restart, TaskHandle, TaskName, Tasks};
use fresnel_net::{Signal, WallClock};
use k8s_openapi::api::core::v1::Pod;

use super::LogSettings;
use super::request::LogRequest;
use super::ring::Lines;
use super::source::Opener;
use super::view::{LogView, PodsState};
use crate::cluster::Gateway;
use crate::index::{Index, Owners};
use crate::published::{Bin, Held, Notice, Publisher, Reader};
use crate::store::Snapshot;

/// What the views of one cluster share.
#[derive(Clone)]
pub struct SessionParts {
    /// Starts the task of a view and of each source.
    pub tasks: Tasks,
    /// The pods of the cluster.
    pub pods: Reader<Snapshot<Pod>>,
    /// The owner index of the cluster, for the pods of a workload.
    pub owners: Reader<Index<Owners>>,
    /// The signal of the cluster: a wake, a network change and "retry now" end a pause.
    pub signal: Signal,
    /// The wall clock, for the time of the next attempt of a source.
    pub clock: WallClock,
    /// The settings.
    pub settings: LogSettings,
    /// The pauses before a task of a view or of a source starts again after a fault.
    pub restart: Backoff,
    /// Frees the values that the readers held.
    pub bin: Bin,
    /// Hears each new value of a view.
    pub notice: Notice,
}

/// One running log view. Its task and its streams stop when this value drops.
pub struct LogSession {
    request: LogRequest,
    publisher: Publisher<LogView>,
    _task: TaskHandle,
}

impl LogSession {
    /// Starts the view of `request` on the log channel of its cluster.
    pub fn start(request: LogRequest, parts: SessionParts, gateway: Gateway) -> Self {
        Self::start_with(request, parts, gateway)
    }

    /// The same with an other opener of streams: a test gives a script.
    pub(super) fn start_with<O: Opener>(request: LogRequest, parts: SessionParts, opener: O) -> Self {
        let first = LogView {
            request: request.clone(),
            pods: PodsState::Loading,
            sources: Arc::new([]),
            lines: Lines::default(),
            dropped: 0,
            bytes: 0,
            unstreamed: 0,
            restarts: 0,
        };
        let publisher = Publisher::new(first, parts.bin.clone(), parts.notice.clone());
        let name = TaskName::scoped("log view", request.cluster.as_str());
        let (restart, tasks) = (Restart::Always(parts.restart), parts.tasks.clone());
        let task = tasks.start(name, restart, {
            let (request, parts, publisher) = (Arc::new(request.clone()), Arc::new(parts), publisher.clone());
            // Each run after the first one follows an internal error of the app.
            let mut runs = 0_u32;
            move || {
                runs = runs.saturating_add(1);
                run::run(Arc::clone(&request), Arc::clone(&parts), opener.clone(), publisher.clone(), runs - 1)
            }
        });
        Self { request, publisher, _task: task }
    }

    /// What the view asked for.
    pub fn request(&self) -> &LogRequest {
        &self.request
    }

    /// The view at this moment.
    pub fn view(&self) -> Held<LogView> {
        self.publisher.read()
    }
}

impl std::fmt::Debug for LogSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LogSession").field("request", &self.request).finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests;
