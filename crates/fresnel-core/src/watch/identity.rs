//! The identity of the cluster behind the connections: the UID of its Namespace `kube-system`.
//!
//! A cluster can be an other one after a lost connection: a restore of a backup, or a new
//! cluster at the same address. Each store compares the version of the server and the UID of
//! one object of its kind with its own, and a new cluster can pass both: its version can be
//! past the cursor, and the first object of a kind can have no namesake in the store. The
//! Namespace `kube-system` comes with each cluster and keeps its UID for its life, so the task
//! of a cluster reads it after each new connection, and an other UID gives the cluster a new
//! number, at which each store lists again. A watch on a connection after those of the last
//! list or check of its store waits for that read (`stream`): a soak saw stores "live" with
//! the objects of the old cluster for 20 s while the read was on its way.

use std::sync::Arc;
use std::time::Duration;

use kube::core::params::GetParams;
use tokio::sync::watch;
use tokio::time::sleep;

use super::lanes::{Identity, Lane, LaneStates, Lanes};
use crate::cluster::Gateway;
use crate::error::CoreError;
use crate::kinds::{Kind, Meta};

/// The Namespace of the system of each cluster.
const SYSTEM: &str = "kube-system";
/// The pause before the next read after a read that failed. The stores after a lost
/// connection wait for the read, and a new connection ends the pause.
const RETRY: Duration = Duration::from_secs(1);

/// The body of the task of the identity of one cluster.
pub(crate) async fn identify(gateway: Gateway, states: watch::Sender<LaneStates>) -> Result<(), CoreError> {
    follow(|| system_uid(&gateway), &states).await
}

/// Reads the UID of the Namespace `kube-system`. `None` for an answer with no UID.
async fn system_uid(gateway: &Gateway) -> Result<Option<Arc<str>>, CoreError> {
    let request = kube::core::Request::new(Kind::Namespace.info().path()).get_metadata(SYSTEM, &GetParams::default());
    let request = request.map_err(|error| CoreError::invalid(format!("the request of {SYSTEM}: {error}")))?;
    let namespace: Meta = gateway.get(request).await?;
    Ok(namespace.metadata.uid.map(Arc::from))
}

/// Reads the UID with `read` once the requests can go out and a connection opened after the
/// start of the last read. An other UID than the one before gives the cluster a new number. A
/// read that failed comes again after a pause, which a new connection ends; the stores wait.
async fn follow<F>(read: impl Fn() -> F, states: &watch::Sender<LaneStates>) -> Result<(), CoreError>
where
    F: Future<Output = Result<Option<Arc<str>>, CoreError>>,
{
    let mut lanes = Lanes::new(states.subscribe());
    let mut known: Option<Arc<str>> = None;
    // True when the next read cannot wait for a new connection. A new run of the task knows
    // no UID, so it reads at once.
    let mut due = true;
    loop {
        let connects = lanes.unread(Lane::Requests, due).await?;
        let (other, again) = match read().await {
            Ok(Some(uid)) => (known.replace(Arc::clone(&uid)).is_some_and(|before| before != uid), false),
            // Each cluster has `kube-system` for its life: one with none is an other cluster in
            // its start, and the next read takes the UID of its own.
            Err(error) if error.status() == Some(404) => {
                let other = known.take().is_some();
                (other, other)
            }
            // No check is possible: a user who cannot read the Namespace, or an answer with no
            // UID. The stores still check the version and one object of their own.
            Ok(None) => (false, false),
            Err(error) if error.status() == Some(403) => (false, false),
            Err(_) => {
                due = true;
                tokio::select! {
                    () = sleep(RETRY) => {}
                    () = lanes.connected_after(connects) => {}
                }
                continue;
            }
        };
        states.send_modify(|states| {
            let number = states.identity.number.wrapping_add(u32::from(other));
            states.identity = Identity { number, checked: connects };
        });
        due = again;
        if again {
            sleep(RETRY).await;
        }
    }
}

#[cfg(test)]
mod tests;
