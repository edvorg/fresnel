//! The ends of a session that the user does not start in its tab: a cluster that leaves the
//! selection, and the quit of the app. Neither may leave a program of the app in the cluster.

use fresnel_core::app::{Command, Health};
use fresnel_core::cluster::ClusterId;

use crate::bed::{Bed, CLUSTER, eventually};
use crate::common::in_time;
use crate::made::{node_shell, node_shells};
use crate::screen::keys;
use crate::shell::{close, no_session_process, shell};

/// A cluster that leaves the selection closes the sessions of its terminals: each session
/// holds the gateway of its cluster, so its hangup still goes. The tab stays with its final
/// state until the user closes it.
pub(crate) async fn a_cluster_that_leaves_the_selection_ends_the_shells_of_its_terminals(bed: &Bed) {
    let view = shell(bed).await;
    keys(&view.terminal, "/bin/sleep 1006 & echo away-$((2*5))\r");
    bed.screen(&view.terminal, "away-10").await;
    select(bed, false);
    bed.until("the final state", |_| view.terminal.state().is_final()).await;
    no_session_process(bed, "shell").await;
    assert!(bed.core.view().terminals.iter().any(|terminal| terminal.id == view.id), "the tab stays");
    select(bed, true);
    bed.until("the cluster is live again", live).await;
    close(bed, &view).await;
}

/// The quit sequence closes each terminal and waits for their ends before the stop of the
/// core: a stop alone would leave the shell and its job, and the privileged pod of a node
/// shell. It comes after the silence, so it waits until the cluster is live again.
pub(crate) async fn the_quit_ends_each_shell_and_deletes_each_node_shell_pod(bed: &Bed) {
    bed.until("the cluster is live again", live).await;
    let view = shell(bed).await;
    keys(&view.terminal, "/bin/sleep 1007 & echo quit-$((4*4))\r");
    bed.screen(&view.terminal, "quit-16").await;
    bed.open(node_shell(bed).await).await;
    assert_eq!(node_shells(bed).await.lines().count(), 1, "the pod of the node shell runs");
    let ended = in_time("the close of each terminal", bed.core.close_terminals()).await;
    assert!(ended, "each session ends within the limit of the quit");
    bed.core.stop();
    no_session_process(bed, "shell").await;
    eventually("the pod of the node shell is gone", || async { node_shells(bed).await.trim().is_empty() }).await;
}

fn select(bed: &Bed, selected: bool) {
    bed.core.send(Command::Select { cluster: ClusterId::new(CLUSTER), selected });
}

fn live(view: &fresnel_core::app::CoreView) -> bool {
    view.clusters.iter().any(|row| row.id.as_str() == CLUSTER && row.health() == Health::Live)
}
