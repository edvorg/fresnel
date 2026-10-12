//! The task of the proxy: it accepts connections, owns their tasks and resets them.

use std::io;
use std::sync::Arc;

use tokio::sync::watch;
use tokio::task::JoinSet;
use tokio::time::{Instant, sleep_until};

use super::ProxyMode;
use super::relay;
use super::state::Shared;
use crate::background::connection_end;

/// Runs until an error. One task owns all connection tasks, so a reset is one operation,
/// and the end of the proxy ends each connection.
pub(super) async fn run(shared: Arc<Shared>) -> io::Result<()> {
    let mut connections = JoinSet::new();
    let reset_order = watch::Sender::new(0_u64);
    let mut mode = shared.mode.subscribe();
    let mut resets_asked = shared.resets_asked.subscribe();
    // A request can come while no driver runs. Without this mark, its test waits with no end.
    resets_asked.mark_changed();
    let mut timed_reset = next_timed_reset(*mode.borrow_and_update());
    loop {
        tokio::select! {
            accepted = shared.listener.accept() => {
                let (client, _) = accepted?;
                let admission = shared.admit();
                connections.spawn(relay::serve(Arc::clone(&shared), client, admission, reset_order.subscribe()));
            }
            Some(ended) = connections.join_next() => connection_end(ended)?,
            Ok(()) = resets_asked.changed() => {
                let request = *resets_asked.borrow_and_update();
                reset_all(&shared, &reset_order, &mut connections).await?;
                shared.resets_done.send_replace(request);
            }
            () = wait_for(timed_reset) => {
                reset_all(&shared, &reset_order, &mut connections).await?;
                timed_reset = next_timed_reset(*mode.borrow());
            }
            Ok(()) = mode.changed() => timed_reset = next_timed_reset(*mode.borrow_and_update()),
        }
    }
}

/// Gives each connection task the order to reset its sockets, and waits for the end of each
/// one. The proxy accepts no connection in this time, so no new connection gets the order.
async fn reset_all(shared: &Shared, reset_order: &watch::Sender<u64>, connections: &mut JoinSet<()>) -> io::Result<()> {
    let open = shared.counters.borrow().open;
    reset_order.send_modify(|order| *order += 1);
    while let Some(ended) = connections.join_next().await {
        connection_end(ended)?;
    }
    shared.count(|counters| counters.reset += open);
    Ok(())
}

fn next_timed_reset(mode: ProxyMode) -> Option<Instant> {
    match mode {
        ProxyMode::ResetEach(period) => Some(Instant::now() + period),
        ProxyMode::Pass | ProxyMode::HoldNew { .. } | ProxyMode::Freeze | ProxyMode::Latency(_) => None,
    }
}

async fn wait_for(time: Option<Instant>) {
    match time {
        Some(time) => sleep_until(time).await,
        None => std::future::pending().await,
    }
}
