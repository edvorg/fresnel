//! The memory account of the core (`crate::memory`): the journal, the tasks, the jobs, the
//! forwards and the terminals, and the stores, indexes, metrics, log views, permissions and
//! connections of each connected cluster.
//!
//! The walk reads a copy of the state, as a view does, and a short lock for what no view
//! has: the stores on demand and the commands that wait. The copies of the objects of the
//! stores then take their time outside each lock.

use fresnel_net::ClusterStatus;
use fresnel_net::task::TaskStatus;

use super::Core;
use super::data::ClusterData;
use super::jobs::JobView;
use crate::cluster::ClusterId;
use crate::index;
use crate::logs::SourceView;
use crate::memory::{Account, Part, Scale};
use crate::search::Source;

impl Core {
    /// The size of each bounded part of the core now (design rule 12). `scale` measures
    /// the objects of the stores, whose size the core does not know.
    pub fn memory(&self, scale: &dyn Scale) -> Vec<Part> {
        let view = self.view();
        let (commands, on_demand) = {
            let state = self.inner.state();
            let on_demand: Vec<(ClusterId, (Vec<Source>, usize))> =
                state.connected.iter().map(|(id, connected)| (id.clone(), connected.memory())).collect();
            (state.commands.len(), on_demand)
        };
        let mut account = Account::new(scale);
        let (entries, bytes) = self.inner.journal.size();
        account.add("journal", entries, bytes);
        let tasks = self.inner.tasks.snapshot().len();
        account.add("tasks", tasks, tasks * size_of::<TaskStatus>());
        account.add("commands", commands, commands * size_of::<super::Command>());
        let jobs = view.jobs.jobs.len();
        account.add("jobs", jobs, jobs * size_of::<JobView>());
        account.add("forwards", view.forwards.len(), view.forwards.len() * size_of::<super::ForwardRow>());
        account.add("terminals", view.terminals.len(), 0);
        for row in &view.clusters {
            account.scoped(row.id.as_str(), |account| {
                if let Some(status) = row.status() {
                    connections(status, account);
                }
                if let Some(data) = &row.data {
                    cluster(data, account);
                }
                if let Some((_, (sources, lanes))) = on_demand.iter().find(|(id, _)| *id == row.id) {
                    sources.iter().for_each(|source| store("store on demand", source, account));
                    account.add("watch lane readers", *lanes, 0);
                }
            });
        }
        account.parts()
    }
}

/// The status records of the channels of a cluster and its warm upgrade connections.
fn connections(status: &ClusterStatus, account: &mut Account) {
    let groups = [&status.watches, &status.requests, &status.logs];
    let channels: usize = groups.iter().map(|group| group.channels.len()).sum();
    account.add("channels", channels, channels * size_of::<fresnel_net::ChannelStatus>());
    account.add("warm upgrade connections", status.upgrade.warm, 0);
}

/// The stores that are always on, the indexes, the metrics, the log views and the
/// permissions of one connected cluster.
fn cluster(data: &ClusterData, account: &mut Account) {
    data.sources.iter().for_each(|source| store("store", source, account));
    index::account(&data.owners, "owner index", account);
    index::account(&data.references, "reference index", account);
    if let Some(events) = &data.events {
        index::account(events, "event index", account);
    }
    data.metrics.account(account);
    for view in data.logs.iter() {
        account.add("log ring", view.lines.len(), view.bytes);
        account.add("log sources", view.sources.len(), view.sources.len() * size_of::<SourceView>());
    }
    account.add("log saves", data.saves.len(), 0);
    let access = &data.access;
    account.add("access answers", access.rules.len() + access.checks.len(), 0);
    account.add("key views", usize::from(data.keys.is_some()), 0);
}

fn store(role: &str, source: &Source, account: &mut Account) {
    let bytes = source.bytes(account);
    account.add(&format!("{role} {}", source.status.kind.info().name), source.status.items, bytes);
}
