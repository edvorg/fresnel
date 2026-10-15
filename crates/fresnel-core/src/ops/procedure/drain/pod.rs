//! The task of one pod of a drain: the eviction loop of kubectl, then the wait until the
//! pod is gone. The pod store of the cluster tells the end, so the wait sends no request.

use std::pin::pin;
use std::sync::Arc;

use futures_util::future::{Either, select};
use k8s_openapi::api::core::v1::Pod;
use serde_json::Value as Json;
use tokio::time::sleep;

use super::super::super::drain::{InStore, Next, PodState, RETRY, after};
use super::super::super::{Target, delete};
use super::super::Cancel;
use super::super::long::{Seen, look, not_followed};
use super::Board;
use crate::cluster::Gateway;
use crate::kinds::Kind;
use crate::published::Reader;
use crate::store::Snapshot;

/// What the task of one pod needs.
pub(super) struct Seat {
    /// The place of the pod in the list of the drain.
    pub(super) index: usize,
    /// The pod, with the UID of the list: a new pod of its name is an other pod.
    pub(super) target: Target,
    pub(super) gateway: Gateway,
    pub(super) pods: Reader<Snapshot<Pod>>,
    /// The grace period of the drain. `None`: the grace period of the pod.
    pub(super) grace: Option<u32>,
    /// The end of the drain: a cancel or its time limit.
    pub(super) stop: Cancel,
    pub(super) board: Arc<Board>,
}

/// Evicts the pod and waits until it is gone. The last state goes to the board.
pub(super) async fn evict(mut seat: Seat) {
    let state = run(&mut seat).await;
    seat.board.set(seat.index, state);
}

async fn run(seat: &mut Seat) -> PodState {
    let mut attempts = 0;
    loop {
        if seat.stop.is_set() {
            return PodState::Stopped;
        }
        seat.board.set(seat.index, PodState::Evicting);
        let request = match delete::evict(&seat.target, seat.grace) {
            Ok(request) => request,
            Err(error) => return PodState::Failed(error),
        };
        let answer = seat.gateway.send::<Json>(request).await.map(drop);
        let pod = match look(&seat.pods.read(), &seat.target) {
            Seen::Object(pod) if pod.metadata.deletion_timestamp.is_some() => InStore::Ending,
            Seen::Object(_) => InStore::Running,
            Seen::Gone | Seen::NotYet | Seen::Blocked(_) => InStore::Unseen,
        };
        match after(answer, pod, attempts) {
            Next::Wait => break,
            Next::Gone => return PodState::Gone,
            Next::End(state) => return state,
            Next::Again(state) => {
                if let PodState::Blocked { attempts: count, .. } = &state {
                    attempts = *count;
                }
                seat.board.set(seat.index, state);
                if let Either::Right(_) = select(pin!(sleep(RETRY)), pin!(seat.stop.until_set())).await {
                    return PodState::Stopped;
                }
            }
        }
    }
    seat.board.set(seat.index, PodState::Terminating);
    loop {
        match look(&seat.pods.read(), &seat.target) {
            Seen::Gone => return PodState::Gone,
            Seen::Blocked(state) => return PodState::Failed(not_followed(Kind::Pod, &state)),
            Seen::Object(_) | Seen::NotYet => {}
        }
        // The server took the eviction: after the stop of the drain, the pod still ends.
        if let Either::Right(_) = select(pin!(seat.pods.changed()), pin!(seat.stop.until_set())).await {
            return PodState::Terminating;
        }
    }
}
