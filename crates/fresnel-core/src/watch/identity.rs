//! The identity of the cluster behind the connections: the UID of its Namespace `kube-system`.
//!
//! A cluster can be an other one after a lost connection: a restore of a backup, or a new
//! cluster at the same address. Each store compares the version of the server and the UID of
//! one object of its kind with its own, and a new cluster can pass both: its version can be
//! past the cursor, and the first object of a kind can have no namesake in the store. The
//! Namespace `kube-system` comes with each cluster and keeps its UID for its life, so the task
//! of a cluster reads it at the connect and after each lost connection, and an other UID gives
//! the cluster a new number, at which each store lists again.

use std::sync::Arc;
use std::time::Duration;

use kube::core::params::GetParams;
use tokio::sync::watch;
use tokio::time::sleep;

use super::lanes::{Lane, LaneStates, Lanes};
use crate::cluster::Gateway;
use crate::error::CoreError;
use crate::kinds::{Kind, Meta};

/// The Namespace of the system of each cluster.
const SYSTEM: &str = "kube-system";
/// The pause before the next read after a read that failed for a time limit or the network.
const RETRY: Duration = Duration::from_secs(3);

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

/// Reads the UID with `read` once the requests can go out, and again after each return of
/// the connections. An other UID than the one before gives the cluster a new number. A user
/// who cannot read the Namespace, a cluster with none and an answer with no UID get no check:
/// the stores still check the version and one object of their own.
async fn follow<F>(read: impl Fn() -> F, states: &watch::Sender<LaneStates>) -> Result<(), CoreError>
where
    F: Future<Output = Result<Option<Arc<str>>, CoreError>>,
{
    let mut lanes = Lanes::new(states.subscribe());
    let mut known: Option<Arc<str>> = None;
    loop {
        lanes.open(Lane::Requests).await?;
        match read().await {
            Ok(Some(uid)) => {
                if known.as_ref().is_some_and(|known| *known != uid) {
                    states.send_modify(|states| states.cluster = states.cluster.wrapping_add(1));
                }
                known = Some(uid);
                lanes.returned().await;
            }
            Ok(None) => return std::future::pending().await,
            Err(error) if matches!(error.status(), Some(403 | 404)) => return std::future::pending().await,
            Err(_) => sleep(RETRY).await,
        }
    }
}

#[cfg(test)]
mod tests;
