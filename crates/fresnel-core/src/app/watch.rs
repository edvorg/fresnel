//! The task that follows the state of one connected cluster.
//!
//! Each change of a state asks the UI for a repaint. Each change of the one state of the
//! cluster gives a journal entry, so the user can read later what the connection did.

use fresnel_net::task::{Backoff, Restart, TaskHandle, TaskName, Tasks};
use fresnel_net::{Cluster, ClusterStatus};

use super::health::{self, Health};
use super::repaint::Repaint;
use crate::cluster::ClusterId;
use crate::error::CoreError;
use crate::journal::{Journal, Level};

/// Starts the task for one cluster. It stops when the handle drops. The task holds the
/// connections of the cluster open, so the handle drops with the disconnect.
pub(super) fn start(tasks: &Tasks, id: &ClusterId, net: Cluster, journal: Journal, repaint: Repaint) -> TaskHandle {
    let source = id.to_string();
    let name = TaskName::scoped("state watch", id.as_str());
    tasks.start(name, Restart::Always(Backoff::default()), move || {
        follow(net.clone(), source.clone(), journal.clone(), repaint.clone())
    })
}

async fn follow(net: Cluster, source: String, journal: Journal, repaint: Repaint) -> Result<(), CoreError> {
    let mut changes = net.changes();
    let mut before = None;
    loop {
        let status = net.status();
        let now = Health::of_status(&status);
        if before != Some(now) {
            let (level, text) = entry(before, now, &status);
            journal.push(level, source.as_str(), text);
            before = Some(now);
        }
        repaint.request();
        changes.next().await;
    }
}

/// The journal entry for a new state of a connected cluster.
fn entry(before: Option<Health>, now: Health, status: &ClusterStatus) -> (Level, String) {
    let reason = health::reason(status).unwrap_or_else(|| "no reason is known".to_owned());
    match now {
        Health::Live => (Level::Info, "live: a request got an answer on each channel".to_owned()),
        Health::Connecting if before.is_none() => (Level::Info, "the channels connect".to_owned()),
        Health::Connecting => (Level::Warning, format!("a connection was lost ({reason}); it connects again")),
        Health::NoConnection => (Level::Warning, format!("no connection ({reason}); the channel tries again")),
        Health::NoCredential => (Level::Error, format!("no credential ({reason})")),
        // A connected cluster has none of these states.
        Health::Off | Health::Failed => (Level::Info, "the cluster has no connection".to_owned()),
    }
}
