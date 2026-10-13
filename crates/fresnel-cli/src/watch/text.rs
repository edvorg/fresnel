//! The report of the `watch` command, for the terminal.

use std::time::SystemTime;

use fresnel_core::kinds::Tier;
use fresnel_core::net::{ChannelState, ClusterStatus, GroupStatus};
use fresnel_core::store::StoreStatus;

use crate::words::{count, size};

/// The facts of one cluster at the time of a report.
pub(super) struct ClusterReport {
    pub(super) name: String,
    pub(super) connections: ClusterStatus,
    pub(super) stores: Vec<StoreStatus>,
}

/// The width of the name of a kind: the longest name is `ValidatingAdmissionPolicyBinding`.
const KIND: usize = 32;

/// One report: one block for each cluster, with one line for each store.
pub(super) fn render(time: &str, clusters: &[ClusterReport], now: SystemTime) -> String {
    let mut lines = vec![format!("fresnel-cli watch, {time}")];
    for cluster in clusters {
        lines.push(String::new());
        lines.push(headline(cluster));
        lines.extend(cluster.stores.iter().map(|store| line(store, now)));
    }
    lines.join("\n") + "\n"
}

/// The sum of a cluster: its stores, its objects and its connections.
fn headline(cluster: &ClusterReport) -> String {
    let live = cluster.stores.iter().filter(|store| store.state.is_live()).count();
    let objects: usize = cluster.stores.iter().map(|store| store.items).sum();
    let status = &cluster.connections;
    let open = [&status.watches, &status.requests, &status.logs].map(connections).iter().sum::<usize>();
    let streams: usize = status.watches.channels.iter().map(|channel| channel.streams_open).sum();
    format!(
        "cluster {}: {live} of {} live, {}, {}, {}",
        cluster.name,
        size(cluster.stores.len(), "store"),
        size(objects, "object"),
        size(open, "connection"),
        size(streams, "watch stream"),
    )
}

/// The channels of a group that hold a connection now.
fn connections(group: &GroupStatus) -> usize {
    let open = |state: &ChannelState| match state {
        ChannelState::Live | ChannelState::Verifying | ChannelState::WaitsForToken => true,
        ChannelState::Connecting { .. } | ChannelState::Backoff { .. } | ChannelState::Retired => false,
    };
    group.channels.iter().filter(|channel| open(&channel.state)).count()
}

fn line(store: &StoreStatus, now: SystemTime) -> String {
    let info = store.kind.info();
    let tier = match info.tier {
        Tier::A => "A",
        Tier::B => "B",
        Tier::C => "C",
    };
    let age = |time: SystemTime| now.duration_since(time).unwrap_or_default().as_secs();
    // A store that is not live tells for how long: that is the age of what it shows.
    let state =
        if store.state.is_live() { "live".to_owned() } else { format!("{} (for {} s)", store.state, age(store.since)) };
    let data = match store.last_contact {
        Some(time) => format!("last data {} s ago", age(time)),
        None => "no data yet".to_owned(),
    };
    let dropped = match store.dropped {
        0 => String::new(),
        dropped => format!(", {} left for the limit", count(dropped, "object")),
    };
    format!("  {:<KIND$}  {tier}  {:>7}  {state}, {data}{dropped}", info.name, store.items)
}

#[cfg(test)]
mod tests;
