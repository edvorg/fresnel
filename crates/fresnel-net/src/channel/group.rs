//! The channels of one purpose in a cluster: watches, requests or log streams.
//!
//! A group starts with one channel. When each channel is at its stream budget, the group
//! opens one more, to a maximum. Only then a request waits for a free stream, and the
//! state of the group shows the count of requests that wait.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tokio::sync::Notify;

use super::streams::StreamPermit;
use super::{Channel, ChannelSettings, ChannelStatus, Parts, Purpose};

/// A copy of the facts of one group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupStatus {
    /// The channels of the group. The first one is always present.
    pub channels: Vec<ChannelStatus>,
    /// The count of requests that wait for a free stream.
    pub waiting: usize,
}

pub(crate) struct Group {
    purpose: Purpose,
    settings: ChannelSettings,
    parts: Parts,
    channels: Mutex<Vec<Channel>>,
    /// Wakes each request that waits for a free stream.
    freed: Arc<Notify>,
    waiting: AtomicUsize,
}

impl Group {
    /// Starts the first channel of the group. It connects at once.
    pub(crate) fn start(parts: Parts, purpose: Purpose, settings: ChannelSettings) -> Self {
        let freed = Arc::new(Notify::new());
        // hyper permits 100 streams before the server sent its limit.
        let first = Channel::start(&parts, purpose, &settings, false, settings.budget(100), Arc::clone(&freed));
        Self { purpose, settings, parts, channels: Mutex::new(vec![first]), freed, waiting: AtomicUsize::new(0) }
    }

    /// A free stream on one channel of the group. The caller sets the time limit of the wait.
    pub(crate) async fn stream(&self) -> (Channel, StreamPermit) {
        if let Some(taken) = self.try_take() {
            return taken;
        }
        let _waiting = Waiting::of(&self.waiting, &self.parts);
        loop {
            let freed = self.freed.notified();
            tokio::pin!(freed);
            // The registration comes before the check, so that no free stream is lost.
            freed.as_mut().enable();
            if let Some(taken) = self.try_take() {
                return taken;
            }
            freed.await;
        }
    }

    fn try_take(&self) -> Option<(Channel, StreamPermit)> {
        let mut channels = self.channels();
        channels.retain(|channel| !channel.shared.streams.is_retired());
        let taken = channels.iter().find_map(|channel| Some((channel.clone(), channel.shared.streams.try_open()?)));
        if taken.is_some() || channels.len() >= self.settings.max_connections.max(1) {
            return taken;
        }
        // The limit of the server is known from the first channel. Without it, a burst of
        // requests could open more streams on the new connection than the server permits.
        let budget = channels.first().map_or(1, |first| first.shared.streams.budget());
        let extra = Channel::start(&self.parts, self.purpose, &self.settings, true, budget, Arc::clone(&self.freed));
        let permit = extra.shared.streams.try_open()?;
        channels.push(extra.clone());
        Some((extra, permit))
    }

    pub(crate) fn status(&self) -> GroupStatus {
        let channels =
            self.channels().iter().filter(|channel| !channel.shared.streams.is_retired()).cloned().collect::<Vec<_>>();
        GroupStatus {
            channels: channels.iter().map(Channel::status).collect(),
            waiting: self.waiting.load(Ordering::Relaxed),
        }
    }

    /// Each use holds the lock for one small change or one copy.
    fn channels(&self) -> MutexGuard<'_, Vec<Channel>> {
        self.channels.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// One request in the count of requests that wait. The count is also correct when the
/// caller drops the wait.
struct Waiting<'a>(&'a AtomicUsize, &'a Parts);

impl<'a> Waiting<'a> {
    fn of(count: &'a AtomicUsize, parts: &'a Parts) -> Self {
        count.fetch_add(1, Ordering::Relaxed);
        parts.changed.notify();
        Self(count, parts)
    }
}

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
        self.1.changed.notify();
    }
}
