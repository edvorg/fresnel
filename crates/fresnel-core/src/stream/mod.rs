//! The streams of a pod over a WebSocket: exec and attach (plan section 12.8), and the
//! tunnels of a port forward (plan section 12.3).
//!
//! Each one opens with [`Gateway::upgrade`](crate::cluster::Gateway::upgrade) on a warm or
//! a new upgrade connection, and runs on a socket of the connection layer, which has the ping,
//! the bounded queues, the states and the counters. This module has the channel protocols.
//! The port forward and the AttachedProcess of kube-rs have no ping and lose data at a close.
//!
//! - `channel`: the channels of a message. `status`: the exit of a process.
//! - `exec`: exec and attach. `forward`: the tunnel of one local connection.

mod channel;
mod exec;
mod forward;
mod status;

pub use exec::{Exec, ExecEnd, ExecSession, Input, Output};
pub use forward::{Tunnel, TunnelEnd, TunnelWriter};
pub use status::Exit;

use fresnel_net::OnEnd;

use crate::cluster::ClusterId;
use crate::journal::Journal;

/// The report of the end of a socket: a journal entry for an abnormal end. A clean end has
/// none, so a forward with many short tunnels does not fill the journal.
fn journaled(journal: &Journal, cluster: &ClusterId, what: &str) -> OnEnd {
    let (journal, cluster, what) = (journal.clone(), cluster.clone(), what.to_owned());
    Box::new(move |end| {
        if !end.is_clean() {
            journal.warning(cluster.as_str(), format!("{what}: the WebSocket ended: {end}"));
        }
    })
}
