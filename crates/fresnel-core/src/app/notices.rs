//! The notices of the system (plan section 11.6): a failed operation, and a cluster that
//! stays lost while the window is hidden. The core decides when a notice goes; the program
//! shows it as a macOS notification through the function of `Core::set_notifier`.
//!
//! A user who looks at the window sees each state there, so a lost cluster gives a notice
//! only while the window is hidden, and one notice for each loss. A failed operation gives
//! a notice in each case: macOS shows no banner over the window that has the focus.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::time::Instant;

use super::health::Health;
use super::jobs::JobsView;
use super::{ClusterRow, Inner};
use crate::cluster::{ClusterId, Registry};
use crate::config::Config;
use crate::config::table::Setting;
use crate::error::CoreError;

/// The time between two looks at the jobs and the clusters.
pub(super) const PERIOD: Duration = Duration::from_secs(1);

/// One notice of the system.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub title: String,
    pub body: String,
}

/// The function that shows a notice.
pub type Notifier = Arc<dyn Fn(Notice) + Send + Sync>;

/// What the core knows of the states that gave a notice.
#[derive(Debug, Default)]
pub(super) struct Notices {
    /// The failed jobs that gave their notice. A job leaves the list of the core after
    /// some time; its number then leaves this set.
    told: BTreeSet<u64>,
    /// Each lost cluster with the start of its loss, and true after its notice.
    lost: BTreeMap<ClusterId, (Instant, bool)>,
}

/// One look of the core: its jobs, the health of each selected cluster with its reason,
/// and whether the window is hidden.
pub(super) struct Look<'a> {
    pub(super) jobs: &'a JobsView,
    pub(super) clusters: Vec<(ClusterId, Health, Option<String>)>,
    pub(super) hidden: bool,
    pub(super) now: Instant,
}

impl Notices {
    /// The notices of one look.
    pub(super) fn step(&mut self, config: &Config, look: Look<'_>) -> Vec<Notice> {
        let mut notices = Vec::new();
        let failed = look.jobs.jobs.iter().filter(|job| job.report.state.error().is_some());
        let numbers: BTreeSet<u64> = failed.clone().map(|job| job.number).collect();
        for job in failed.filter(|job| !self.told.contains(&job.number)) {
            if config.flag(Setting::NotifyFailed) {
                let target = &job.order.target;
                let (kind, key) = (target.kind.info().name, &target.key);
                let object = if key.namespace.is_empty() {
                    key.name.to_string()
                } else {
                    format!("{}/{}", key.namespace, key.name)
                };
                let error = job.report.state.error().map(ToString::to_string).unwrap_or_default();
                let title = format!("{} failed in {}", job.order.operation.label(), target.cluster);
                notices.push(Notice { title, body: format!("{kind} {object}: {error}") });
            }
        }
        self.told = numbers;
        self.lost.retain(|id, _| look.clusters.iter().any(|(cluster, health, _)| cluster == id && is_lost(*health)));
        let after = config.duration(Setting::LostAfter);
        for (id, health, reason) in look.clusters.iter().filter(|(_, health, _)| is_lost(*health)) {
            let (since, told) = self.lost.entry(id.clone()).or_insert((look.now, false));
            if look.hidden && !*told && look.now.duration_since(*since) >= after && config.flag(Setting::NotifyLost) {
                *told = true;
                let reason = reason.clone().unwrap_or_else(|| format!("{health:?}"));
                let lost = format!("lost for {} s", look.now.duration_since(*since).as_secs());
                notices.push(Notice { title: format!("{id} is {lost}"), body: reason });
            }
        }
        notices
    }
}

/// True for a cluster that had a connection to lose: a cluster that connects for the first
/// time is not lost, nor one that is off.
fn is_lost(health: Health) -> bool {
    matches!(health, Health::Failed | Health::NoConnection | Health::NoCredential)
}

/// The task of the notices: a look each period, and each notice to the notifier and to
/// the journal, so the log file has each one. `memory` outlives a run of the task, so a
/// run after a panic gives no notice a second time.
pub(super) async fn run(inner: Arc<Inner>, memory: Arc<Mutex<Notices>>) -> Result<(), CoreError> {
    loop {
        tokio::time::sleep(PERIOD).await;
        let (config, jobs, selected) = {
            let state = inner.state();
            (Arc::clone(&state.config), state.jobs.view(), state.selected.clone())
        };
        let clusters = inner.registry.get().map(Registry::snapshot).unwrap_or_default();
        let clusters = clusters.into_iter().filter(|cluster| selected.contains(&cluster.id)).map(|cluster| {
            let row = ClusterRow::new(cluster, true, None);
            (row.id.clone(), row.health(), row.reason())
        });
        let hidden = inner.hidden.load(Ordering::Relaxed);
        let look = Look { jobs: &jobs, clusters: clusters.collect(), hidden, now: Instant::now() };
        let found = memory.lock().unwrap_or_else(PoisonError::into_inner).step(&config, look);
        let notifier = inner.notifier.lock().unwrap_or_else(PoisonError::into_inner).clone();
        for notice in found {
            inner.journal.info(SOURCE, format!("{}: {}", notice.title, notice.body));
            if let Some(notifier) = &notifier {
                notifier(notice);
            }
        }
    }
}

/// The source of the journal entries of the notices.
const SOURCE: &str = "notice";

#[cfg(test)]
mod tests;
