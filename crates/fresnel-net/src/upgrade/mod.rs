//! Upgrade connections: the HTTP/1.1 client for exec, attach and port forward (plan section 8.9).
//!
//! An upgrade needs its own HTTP/1.1 connection. While a forward or a terminal is open, the
//! warm set keeps idle connections ready, so that a new one does not wait for a connect.
//! An idle HTTP/1.1 connection has no ping. Thus each warm connection gets a test before
//! its use, and a wake or a network change drops the set.

mod line;

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use bytes::Bytes;
use futures_util::future::join_all;
use http::{Request, Response};
use http_body_util::Full;
use tokio::sync::Notify;
use tokio::time::sleep;

use crate::body::ResponseBody;
use crate::channel::Parts;
use crate::connect::dial::Protocol;
use crate::error::NetError;
use crate::task::{Backoff, Restart, TaskHandle, TaskName};
use line::Line;

/// The settings of the upgrade client.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpgradeSettings {
    /// The fewest idle connections while a forward or a terminal is open.
    pub warm_min: usize,
    /// The most idle connections.
    pub warm_max: usize,
    /// Each warm connection sends a light request each time this period ends. A
    /// kube-apiserver closes a connection with no request after 90 s.
    pub refresh_period: Duration,
    /// The time limit of a light request in a refresh.
    pub refresh_limit: Duration,
    /// The time limit of the test of a warm connection before its use. After this time,
    /// the request uses a new connection.
    pub test_limit: Duration,
}

impl Default for UpgradeSettings {
    fn default() -> Self {
        Self {
            warm_min: 2,
            warm_max: 4,
            refresh_period: Duration::from_secs(30),
            refresh_limit: Duration::from_secs(10),
            test_limit: Duration::from_secs(1),
        }
    }
}

/// A copy of the facts of the upgrade client of one cluster.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UpgradeStatus {
    /// The idle connections that are ready now.
    pub warm: usize,
    /// The forwards and terminals that keep the set warm.
    pub holders: usize,
    /// Requests that used a warm connection.
    pub reused: u64,
    /// Requests that used a new connection.
    pub opened: u64,
    /// Warm connections that failed a test or that a wake dropped.
    pub dropped: u64,
    /// Why the last connect for the warm set failed. `None` after a connect that worked.
    pub last_error: Option<String>,
}

struct Shared {
    settings: UpgradeSettings,
    parts: Parts,
    idle: Mutex<Vec<Line>>,
    holders: AtomicUsize,
    /// Wakes the keeper: a holder came or left, or a request took a connection.
    changed: Notify,
    reused: AtomicU64,
    opened: AtomicU64,
    dropped: AtomicU64,
    fill_error: Mutex<Option<String>>,
}

/// The upgrade client of one cluster.
pub(crate) struct UpgradeClient {
    shared: Arc<Shared>,
    _keeper: TaskHandle,
}

/// Keeps the warm set filled while it lives. A forward and a terminal each hold one.
#[derive(Debug)]
pub struct WarmGuard {
    shared: Arc<Shared>,
}

impl Drop for WarmGuard {
    fn drop(&mut self) {
        self.shared.holders.fetch_sub(1, Ordering::Relaxed);
        self.shared.changed.notify_one();
    }
}

impl UpgradeClient {
    pub(crate) fn start(parts: Parts, settings: UpgradeSettings) -> Self {
        let name = TaskName::scoped("warm upgrade connections", Arc::clone(&parts.cluster));
        let tasks = parts.tasks.clone();
        let shared = Arc::new(Shared {
            settings,
            parts,
            idle: Mutex::new(Vec::new()),
            holders: AtomicUsize::new(0),
            changed: Notify::new(),
            reused: AtomicU64::new(0),
            opened: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            fill_error: Mutex::new(None),
        });
        let body = {
            let shared = Arc::clone(&shared);
            move || Arc::clone(&shared).keep()
        };
        let keeper = tasks.start(name, Restart::Always(Backoff::default()), body);
        Self { shared, _keeper: keeper }
    }

    /// Sends one upgrade request on a warm connection that passed its test now, or on a new
    /// connection. The caller sets the time limit.
    pub(crate) async fn send(&self, request: Request<Full<Bytes>>) -> Result<Response<ResponseBody>, NetError> {
        let shared = &self.shared;
        let line = match shared.tested_warm_line().await {
            Some(line) => line,
            None => {
                shared.opened.fetch_add(1, Ordering::Relaxed);
                Line::open(shared.parts.dial.as_ref()).await?
            }
        };
        shared.changed.notify_one();
        line.send(request).await
    }

    pub(crate) fn keep_warm(&self) -> WarmGuard {
        self.shared.holders.fetch_add(1, Ordering::Relaxed);
        self.shared.changed.notify_one();
        WarmGuard { shared: Arc::clone(&self.shared) }
    }

    pub(crate) fn status(&self) -> UpgradeStatus {
        UpgradeStatus {
            warm: self.shared.idle().len(),
            holders: self.shared.holders.load(Ordering::Relaxed),
            reused: self.shared.reused.load(Ordering::Relaxed),
            opened: self.shared.opened.load(Ordering::Relaxed),
            dropped: self.shared.dropped.load(Ordering::Relaxed),
            last_error: self.shared.fill_error.lock().unwrap_or_else(PoisonError::into_inner).clone(),
        }
    }
}

impl Shared {
    /// A warm connection that answered a light request now. One that fails its test puts
    /// each other warm connection in doubt, so the set drops.
    async fn tested_warm_line(&self) -> Option<Line> {
        let mut line = self.idle().pop()?;
        let request = (self.parts.light_request)(Protocol::Http1);
        if line.test(request, self.settings.test_limit).await.is_ok() {
            self.reused.fetch_add(1, Ordering::Relaxed);
            return Some(line);
        }
        self.dropped.fetch_add(1, Ordering::Relaxed);
        self.drop_idle();
        None
    }

    fn drop_idle(&self) {
        let dropped = std::mem::take(&mut *self.idle());
        self.dropped.fetch_add(u64::try_from(dropped.len()).unwrap_or(u64::MAX), Ordering::Relaxed);
        self.parts.changed.notify();
    }

    /// The count of idle connections to keep: one more than the open forwards and terminals.
    fn target(&self) -> usize {
        match self.holders.load(Ordering::Relaxed) {
            0 => 0,
            holders => (holders + 1).clamp(self.settings.warm_min, self.settings.warm_max.max(self.settings.warm_min)),
        }
    }

    /// The task of the warm set. It never ends by itself.
    async fn keep(self: Arc<Self>) -> Result<(), NetError> {
        let mut listener = self.parts.signal.listen();
        // One timer for all turns of the loop. A timer that started again in each turn
        // would never end while requests take connections faster than the period.
        let next_refresh = sleep(self.settings.refresh_period);
        tokio::pin!(next_refresh);
        loop {
            self.fill().await;
            tokio::select! {
                () = &mut next_refresh => {
                    self.refresh().await;
                    next_refresh.set(sleep(self.settings.refresh_period));
                }
                () = self.changed.notified() => {}
                // After a wake or a network change, each idle connection can be silent.
                _cause = listener.raised() => self.drop_idle(),
            }
        }
    }

    /// Opens connections until the set has its target, and closes those above the target.
    async fn fill(&self) {
        let target = self.target();
        let missing = {
            let mut idle = self.idle();
            if idle.len() > target {
                idle.truncate(target);
                self.parts.changed.notify();
            }
            target - idle.len()
        };
        if missing == 0 {
            return;
        }
        // A connect that fails leaves a gap, and its error shows in the state. The next
        // run of the keeper fills the gap.
        let opened = join_all((0..missing).map(|_| Line::open(self.parts.dial.as_ref()))).await;
        let mut error = None;
        for line in opened {
            match line {
                Ok(line) => self.idle().push(line),
                Err(failed) => error = Some(failed.to_string()),
            }
        }
        *self.fill_error.lock().unwrap_or_else(PoisonError::into_inner) = error;
        self.parts.changed.notify();
    }

    /// Sends a light request on each idle connection. One that gives no answer goes.
    async fn refresh(&self) {
        let lines = std::mem::take(&mut *self.idle());
        let limit = self.settings.refresh_limit;
        let tested = join_all(lines.into_iter().map(|mut line| async move {
            let request = (self.parts.light_request)(Protocol::Http1);
            line.test(request, limit).await.ok().map(|()| line)
        }))
        .await;
        let before = tested.len();
        let good: Vec<Line> = tested.into_iter().flatten().collect();
        self.dropped.fetch_add(u64::try_from(before - good.len()).unwrap_or(u64::MAX), Ordering::Relaxed);
        self.idle().extend(good);
    }

    /// Each use holds the lock for one small change, and never across a wait.
    fn idle(&self) -> MutexGuard<'_, Vec<Line>> {
        self.idle.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl std::fmt::Debug for Shared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpgradeClient").field("warm", &self.idle().len()).finish_non_exhaustive()
    }
}
