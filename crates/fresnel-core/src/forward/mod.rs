//! Port forwards that detect a break and recover (plan section 12.3).
//!
//! A [`Forward`] owns one task with the local listener of its rule. The listener stays open
//! during each break. Each local connection gets a tunnel of its own (`stream::Tunnel`) to
//! the target pod, which the stores of the cluster choose and change: a pod that stops gives
//! way to a new one. The path to the cluster is the one check of its channels; a tunnel has
//! its own ping. The recovery has no end, and the channels connect again with their backoff;
//! a wake, a network change and "retry now" end their waits, and a ready pod opens the gate.
//!
//! - `rule`: what the user asks for. `resolve`: from the stores to one pod and one port.
//! - `state`: the facts and the state. `view`: what the view shows. `feed`: what a
//!   connected cluster gives. `run`: the task. `conn`: one local connection. `shared`: what
//!   the two share.

mod conn;
mod feed;
mod resolve;
mod rule;
mod run;
mod shared;
mod state;
mod view;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use fresnel_net::WallClock;
use fresnel_net::task::{Backoff, Restart, TaskHandle, TaskName, Tasks};
use tokio::sync::watch;

pub use feed::Feed;
pub use resolve::TargetPod;
pub use rule::{Bind, ForwardRule, LocalPort, TARGETS};
pub use state::ForwardState;
pub use view::{Bytes, Connections, ForwardView, TargetData};

use crate::journal::Journal;
use crate::published::{Bin, Held, Notice, Publisher};
use run::{FeedState, Parts};
use shared::Shared;

/// The times of the forwards. Each one is a setting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForwardSettings {
    /// The longest wait of a local connection for a tunnel during a break, before its reset
    /// (plan section 12.3: 5 s).
    pub wait: Duration,
    /// The time that a tunnel stays after the end of the local input while the pod sends
    /// nothing: the WebSocket protocol has no half close (prototype, problem 7: 1 s).
    pub linger: Duration,
    /// The pause before a local connection tries a tunnel again after a failure.
    pub retry_pause: Duration,
    /// The pauses before the task of a forward starts again after a fault of its own.
    pub restart: Backoff,
}

impl Default for ForwardSettings {
    fn default() -> Self {
        Self {
            wait: Duration::from_secs(5),
            linger: Duration::from_secs(1),
            retry_pause: Duration::from_millis(500),
            restart: Backoff::default(),
        }
    }
}

/// What the owner of the forwards gives to each one.
#[derive(Clone)]
pub struct ForwardParts {
    pub tasks: Tasks,
    /// Gets each step of the state of a forward, and each fault.
    pub journal: Journal,
    pub clock: WallClock,
    /// Frees the views that the readers held.
    pub bin: Bin,
    /// Hears each new view.
    pub notice: Notice,
    pub settings: ForwardSettings,
}

/// One running forward. Its listener closes, and each of its local connections gets a reset,
/// when this value drops.
pub struct Forward {
    publisher: Publisher<ForwardView>,
    feed: watch::Sender<FeedState>,
    retries: watch::Sender<u64>,
    clock: WallClock,
    task: Option<TaskHandle>,
}

impl Forward {
    /// Starts the forward of `rule`. It binds its listener at once and waits for the feed of
    /// its cluster: `feed` gives it, or tells why the cluster gives none now.
    pub fn start(rule: ForwardRule, feed: Result<Arc<Feed>, String>, parts: ForwardParts) -> Self {
        let starting = ForwardState::Starting("the local port binds".to_owned());
        let first = ForwardView::new(rule.clone(), starting, parts.clock.now());
        let publisher = Publisher::new(first, parts.bin, parts.notice);
        let (feed, feeds) = watch::channel(feed);
        let (retries, retries_seen) = watch::channel(0);
        let settings = &parts.settings;
        let times = [settings.wait, settings.linger, settings.retry_pause];
        let shared = Arc::new(Shared::new(parts.journal, times, parts.clock.clone()));
        let name = TaskName::scoped("forward", rule.cluster.as_str());
        let run = Arc::new(Parts {
            rule,
            shared,
            publisher: publisher.clone(),
            feeds,
            retries: retries_seen,
            listener: Mutex::new(None),
            clock: parts.clock.clone(),
        });
        let task = parts.tasks.start(name, Restart::Always(settings.restart), move || run::run(Arc::clone(&run)));
        Self { publisher, feed, retries, clock: parts.clock, task: Some(task) }
    }

    /// The forward at this moment.
    pub fn view(&self) -> Held<ForwardView> {
        self.publisher.read()
    }

    /// Gives the feed of a new connect of the cluster, or why it has none now. The same feed
    /// changes nothing.
    pub fn set_feed(&self, feed: Result<Arc<Feed>, String>) {
        self.feed.send_if_modified(|now| {
            let same = match (&*now, &feed) {
                (Ok(old), Ok(new)) => Arc::ptr_eq(old, new),
                (Err(old), Err(new)) => old == new,
                _ => false,
            };
            if !same {
                *now = feed;
            }
            !same
        });
    }

    /// "Retry now": the cluster tests its path at once, and the failures in a row end.
    pub fn retry(&self) {
        self.retries.send_modify(|count| *count = count.wrapping_add(1));
    }

    /// Stops the forward: its listener closes, and each local connection gets a reset.
    /// Gives its last view, in the state "stopped".
    pub fn stop(mut self) -> Held<ForwardView> {
        self.end();
        self.publisher.read()
    }

    fn end(&mut self) {
        // The task stops first: a run that stops can publish once more before its next wait.
        drop(self.task.take());
        let now = self.clock.now();
        self.publisher.close(|last| ForwardView {
            state: ForwardState::Stopped,
            since: now,
            addresses: Vec::new(),
            target: None,
            connections: Connections { open: 0, ..last.connections },
            ..last.clone()
        });
    }
}

impl Drop for Forward {
    fn drop(&mut self) {
        if self.task.is_some() {
            self.end();
        }
    }
}

impl std::fmt::Debug for Forward {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Forward").field("view", &*self.view()).finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests;
