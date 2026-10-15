//! The drain of a node as a task with steps (plan sections 7.3 and 10, research 08 section
//! 7.2): the cordon, the list of the pods on the node, the verdict of each pod, and then one
//! task for each pod that it evicts. The outcome shows each pod with its state.
//!
//! A pod that blocks stops the drain before its first eviction. A cancel and the time limit
//! stop each task: a wait ends at once, an eviction on its way runs to its answer. The node
//! stays cordoned after each end, as with kubectl; the outcome tells it. After the cordon, no
//! end is "not sent": the cordon is in effect.
//!
//! - `pod`: the task of one pod: its evictions and the wait until it is gone.

mod pod;

use std::collections::{BTreeMap, HashMap};
use std::future::pending;
use std::sync::{Arc, Mutex, PoisonError};

use k8s_openapi::List;
use k8s_openapi::api::core::v1::Pod;
use tokio::sync::Notify;
use tokio::task::JoinSet;
use tokio::time::{Instant, sleep_until};

use super::super::drain::{self, Drain, DrainOptions, DrainPod, Found, PodState, Verdict};
use super::super::{Target, node, request};
use super::long::no_store;
use super::{Cancel, JobState, Outcome, Parts, Progress, Steps, send, written};
use crate::error::CoreError;
use crate::kinds::Kind;
use crate::published::Reader;
use crate::store::{ObjectKey, Snapshot};

/// Drains the node of the target.
pub(super) async fn drain<P: Progress>(
    target: &Target,
    options: &DrainOptions,
    parts: &Parts,
    job: &mut Steps<'_, P>,
) -> Result<(), JobState> {
    let store = parts.pods.clone().ok_or_else(|| no_store(Kind::Pod))?;
    let answer = send(parts, job, "cordon the node", node::cordon(target, true)).await?;
    job.done("the node takes no new pod");
    let mut shown = Drain { cordoned: true, written: written(&answer), pods: Arc::from([]) };
    job.outcome(Outcome::Drain(shown.clone()));

    job.begin("list the pods on the node");
    let list = node::pods_on(&target.key.name).map_err(JobState::Failed)?;
    let listed = parts.gateway.list_page::<List<Pod>>(list).await.map_err(JobState::Failed)?.items;
    on_node(&listed, &target.key.name)?;
    job.done(count_pods(listed.len()));
    let found = daemon_sets(&listed, parts, job).await?;

    job.begin("check each pod");
    let checked: Vec<(Target, Verdict)> = listed
        .iter()
        .map(|pod| {
            let target = of_pod(target, pod);
            let namespace = target.key.namespace.clone();
            let found = |name: &str| {
                // A DaemonSet that the reads above missed is not known to be gone.
                let read = found.get(&(namespace, name.to_owned())).cloned();
                read.unwrap_or_else(|| Found::Unknown("the drain did not read it".to_owned()))
            };
            let verdict = drain::verdict(pod, options, found);
            (target, verdict)
        })
        .collect();
    let pods: Vec<DrainPod> = checked
        .iter()
        .map(|(pod, verdict)| DrainPod {
            key: pod.key.clone(),
            verdict: verdict.clone(),
            state: PodState::first(verdict),
        })
        .collect();
    shown.pods = pods.into();
    let blocks: Vec<(&ObjectKey, &drain::Block)> = checked
        .iter()
        .filter_map(|(pod, verdict)| match verdict {
            Verdict::Block(block) => Some((&pod.key, block)),
            Verdict::Evict(_) | Verdict::Keep(_) => None,
        })
        .collect();
    if !blocks.is_empty() {
        let text = drain::blocked(&blocks);
        shown.pods = stopped(&shown.pods);
        job.outcome(Outcome::Drain(shown));
        return Err(JobState::Failed(CoreError::invalid(text)));
    }
    let evicted: Vec<(usize, Target)> = checked
        .into_iter()
        .enumerate()
        .filter(|(_, (_, verdict))| matches!(verdict, Verdict::Evict(_)))
        .map(|(index, (pod, _))| (index, pod))
        .collect();
    job.done(format!("{} to evict, {} to keep", evicted.len(), shown.pods.len() - evicted.len()));
    job.outcome(Outcome::Drain(shown.clone()));
    if parts.cancel.is_set() {
        return Err(JobState::Cancelled);
    }
    evict_each(evicted, store, options, parts, job, shown).await
}

/// "1 pod", "3 pods".
fn count_pods(count: usize) -> String {
    if count == 1 { "1 pod".to_owned() } else { format!("{count} pods") }
}

/// Fails the drain when the list has a pod of an other node. The server selects the pods of
/// the node; one that drops the field selector, or a proxy that does, would give each pod of
/// the cluster, and the drain would evict them all.
fn on_node(pods: &[Pod], node: &str) -> Result<(), JobState> {
    let others = pods.iter().filter(|pod| pod.spec.as_ref().and_then(|spec| spec.node_name.as_deref()) != Some(node));
    let others = others.count();
    if others == 0 {
        return Ok(());
    }
    let text = format!("the list of the server has {} of an other node, so no eviction went out", count_pods(others));
    Err(JobState::Failed(CoreError::invalid(text)))
}

/// The target of a pod of the list, in the cluster of the node.
fn of_pod(node: &Target, pod: &Pod) -> Target {
    let meta = &pod.metadata;
    let key = ObjectKey::new(meta.namespace.as_deref().unwrap_or_default(), meta.name.as_deref().unwrap_or_default());
    Target { cluster: node.cluster.clone(), kind: Kind::Pod, key, uid: meta.uid.as_deref().unwrap_or_default().into() }
}

/// Reads the DaemonSet of each pod that one controls and runs, as kubectl does: a pod of a
/// DaemonSet that is gone has no controller that starts it again.
async fn daemon_sets<P: Progress>(
    pods: &[Pod],
    parts: &Parts,
    job: &mut Steps<'_, P>,
) -> Result<BTreeMap<(Arc<str>, String), Found>, JobState> {
    let mut found = BTreeMap::new();
    for pod in pods {
        let owners = pod.metadata.owner_references.iter().flatten();
        let owner = owners.into_iter().find(|owner| owner.controller == Some(true) && owner.kind == "DaemonSet");
        let namespace: Arc<str> = pod.metadata.namespace.as_deref().unwrap_or_default().into();
        if let Some(owner) = owner {
            found.insert((namespace, owner.name.clone()), Found::Missing);
        }
    }
    if found.is_empty() {
        return Ok(found);
    }
    job.begin(format!("read the DaemonSets of the pods: {}", found.len()));
    for ((namespace, name), state) in &mut found {
        let request = request::get_named(Kind::DaemonSet, namespace, name).map_err(JobState::Failed)?;
        *state = match parts.gateway.get::<serde_json::Value>(request).await {
            Ok(_) => Found::Here,
            Err(error) if error.status() == Some(404) => Found::Missing,
            Err(error) => Found::Unknown(error.to_string()),
        };
    }
    job.done("read");
    Ok(found)
}

/// The pods of a drain that ended before its evictions: a pod that waited stops.
fn stopped(pods: &[DrainPod]) -> Arc<[DrainPod]> {
    let stop = |pod: &DrainPod| match pod.state {
        PodState::Waiting => DrainPod { state: PodState::Stopped, ..pod.clone() },
        _ => pod.clone(),
    };
    pods.iter().map(stop).collect()
}

/// The pods of a running drain: the task of each pod changes its state, and the job shows
/// the list after each change.
pub(super) struct Board {
    pods: Mutex<Vec<DrainPod>>,
    changed: Notify,
}

impl Board {
    fn set(&self, index: usize, state: PodState) {
        {
            let mut pods = self.pods.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(pod) = pods.get_mut(index) {
                pod.state = state;
            }
        }
        self.changed.notify_one();
    }

    fn copy(&self) -> Arc<[DrainPod]> {
        self.pods.lock().unwrap_or_else(PoisonError::into_inner).iter().cloned().collect()
    }
}

/// Starts the task of each pod to evict, with its place in the list, and follows them to
/// their end, the cancel or the time limit. `pods` is the store whose snapshots tell the end
/// of each pod. The state of the job comes from the pods: a pod that failed fails the drain.
async fn evict_each<P: Progress>(
    evicted: Vec<(usize, Target)>,
    pods: Reader<Snapshot<Pod>>,
    options: &DrainOptions,
    parts: &Parts,
    job: &mut Steps<'_, P>,
    mut shown: Drain,
) -> Result<(), JobState> {
    job.begin(format!("evict {} and wait until each one is gone", count_pods(evicted.len())));
    let board = Arc::new(Board { pods: Mutex::new(shown.pods.to_vec()), changed: Notify::new() });
    let (stop, stopping) = Cancel::pair();
    let mut tasks = JoinSet::new();
    let mut seats = HashMap::new();
    for (index, target) in evicted {
        let seat = pod::Seat {
            index,
            target,
            gateway: parts.gateway.clone(),
            pods: pods.clone(),
            grace: options.grace_period,
            stop: stopping.clone(),
            board: Arc::clone(&board),
        };
        seats.insert(tasks.spawn(pod::evict(seat)).id(), index);
    }
    let deadline = options.timeout.map(|timeout| Instant::now() + timeout);
    let mut cancel = parts.cancel.clone();
    let mut end = None;
    loop {
        let limit = async {
            match deadline {
                Some(deadline) => sleep_until(deadline).await,
                None => pending().await,
            }
        };
        tokio::select! {
            joined = tasks.join_next_with_id() => match joined {
                None => break,
                Some(Ok(_)) => {}
                Some(Err(error)) => {
                    if let Some(index) = seats.get(&error.id()) {
                        board.set(*index, PodState::Failed(CoreError::invalid("the task of the pod stopped with a panic")));
                    }
                }
            },
            () = board.changed.notified() => {
                shown.pods = board.copy();
                job.outcome(Outcome::Drain(shown.clone()));
            }
            () = cancel.until_set(), if end.is_none() => {
                stop.send_replace(true);
                end = Some(JobState::Cancelled);
            }
            () = limit, if end.is_none() => {
                stop.send_replace(true);
                let seconds = options.timeout.unwrap_or_default().as_secs();
                let text = format!("the drain did not end in its time limit of {seconds} s{}", held(&board.copy()));
                end = Some(JobState::Failed(CoreError::invalid(text)));
            }
        }
    }
    shown.pods = board.copy();
    job.outcome(Outcome::Drain(shown.clone()));
    if let Some(end) = end {
        return Err(end);
    }
    outcome_of(&shown.pods)?;
    job.done("each pod is gone");
    Ok(())
}

/// ": the PodDisruptionBudget web holds 2 pods" for each budget that holds a pod now, so the
/// end of a drain at its time limit tells why. The state of the pod changes at the stop.
fn held(pods: &[DrainPod]) -> String {
    let mut budgets = BTreeMap::new();
    for pod in pods.iter() {
        if let PodState::Blocked { budget: Some(budget), .. } = &pod.state {
            *budgets.entry(budget.as_str()).or_insert(0) += 1;
        }
    }
    let texts: Vec<String> = budgets
        .into_iter()
        .map(|(budget, count)| format!("the PodDisruptionBudget {budget} holds {}", count_pods(count)))
        .collect();
    if texts.is_empty() { String::new() } else { format!(": {}", texts.join(", ")) }
}

/// The state of a drain whose tasks all ended: a pod whose eviction has an unknown result
/// makes it unknown, a pod that failed makes it fail.
fn outcome_of(pods: &[DrainPod]) -> Result<(), JobState> {
    let failed = |pod: &DrainPod| -> Option<JobState> {
        let what = |error: &CoreError| error.clone().about(&format!("{}/{}", pod.key.namespace, pod.key.name));
        match &pod.state {
            PodState::Unknown(error) => Some(JobState::Unknown(what(error))),
            PodState::Failed(error) => Some(JobState::Failed(what(error))),
            _ => None,
        }
    };
    let states: Vec<JobState> = pods.iter().filter_map(failed).collect();
    match states.iter().find(|state| matches!(state, JobState::Unknown(_))).or(states.first()) {
        Some(state) => Err(state.clone()),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pod(name: &str, state: PodState) -> DrainPod {
        DrainPod { key: ObjectKey::new("shop", name), verdict: Verdict::Evict(None), state }
    }

    #[test]
    fn the_end_of_the_pods_gives_the_state_of_the_drain_and_a_block_stops_each_pod_that_waited() {
        let error = CoreError::answered(403, "refused");
        assert_eq!(outcome_of(&[pod("a", PodState::Gone), pod("b", PodState::Kept)]), Ok(()));
        let failed = outcome_of(&[pod("a", PodState::Gone), pod("b", PodState::Failed(error.clone()))]);
        assert_eq!(failed, Err(JobState::Failed(CoreError::answered(403, "shop/b: refused"))), "the rules drop at 403");
        let pods = [pod("a", PodState::Failed(error.clone())), pod("b", PodState::Unknown(error))];
        let unknown = JobState::Unknown(CoreError::answered(403, "shop/b: refused"));
        assert_eq!(outcome_of(&pods), Err(unknown), "unknown first");

        let states: Vec<PodState> = stopped(&[pod("a", PodState::Waiting), pod("b", PodState::Kept)])
            .iter()
            .map(|pod| pod.state.clone())
            .collect();
        assert_eq!(states, [PodState::Stopped, PodState::Kept]);
    }
}
