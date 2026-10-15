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
///
/// The connect began with the link of `net`, so the first entry comes here and not from
/// the task: on a near server the channels can be live before the task reads a state,
/// and the journal would then lack the start of the connect.
pub(super) fn start(tasks: &Tasks, id: &ClusterId, net: Cluster, journal: Journal, repaint: Repaint) -> TaskHandle {
    let source = id.to_string();
    journal.info(source.as_str(), "the channels connect");
    let name = TaskName::scoped("state watch", id.as_str());
    tasks.start(name, Restart::Always(Backoff::default()), move || {
        follow(net.clone(), source.clone(), journal.clone(), repaint.clone())
    })
}

/// Each run starts from the state of the connect. A run after a panic thus writes the
/// state of that moment again, unless the cluster still connects.
async fn follow(net: Cluster, source: String, journal: Journal, repaint: Repaint) -> Result<(), CoreError> {
    let mut changes = net.changes();
    let mut before = Health::Connecting;
    loop {
        let status = net.status();
        let now = Health::of_status(&status);
        if before != now {
            let (level, text) = entry(now, &status);
            journal.push(level, source.as_str(), text);
            before = now;
        }
        repaint.request();
        changes.next().await;
    }
}

/// The journal entry for a new state of a connected cluster.
fn entry(now: Health, status: &ClusterStatus) -> (Level, String) {
    let reason = health::reason(status).unwrap_or_else(|| "no reason is known".to_owned());
    match now {
        Health::Live => (Level::Info, "live: a request got an answer on each channel".to_owned()),
        Health::Connecting => (Level::Warning, format!("a connection was lost ({reason}); it connects again")),
        Health::NoConnection => (Level::Warning, format!("no connection ({reason}); the channel tries again")),
        Health::NoCredential => (Level::Error, format!("no credential ({reason})")),
        // A connected cluster has none of these states.
        Health::Off | Health::Failed => (Level::Info, "the cluster has no connection".to_owned()),
    }
}

#[cfg(test)]
mod tests;
