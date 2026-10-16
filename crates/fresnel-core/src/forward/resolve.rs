//! The target of a forward: from the stores of its cluster to one pod and one port (plan
//! section 12.3). Each function reads snapshots only, so a unit test needs no cluster.
//!
//! A Pod target must run. A Service or a workload takes a ready pod, and with no ready pod a
//! pod that runs, which the view marks: a user forwards to such a pod to find its fault
//! (port forward prototype, R7). The pod of now stays while it fits, so a forward does not
//! move with no cause; a new ready pod replaces one that only runs.

use std::sync::Arc;

use k8s_openapi::api::core::v1::{Pod, Service, ServicePort};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;

use super::rule::ForwardRule;
use crate::index::{Index, Owners};
use crate::kinds::Kind;
use crate::store::{ObjectKey, Snapshot};

/// The pod and the port of the next tunnels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetPod {
    pub pod: ObjectKey,
    /// The UID: a pod of a StatefulSet can come again with the same name.
    pub uid: Arc<str>,
    /// The port in the pod.
    pub port: u16,
    /// False for a pod that runs and is not ready: the view tells it.
    pub ready: bool,
}

/// What the stores give for a rule now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Resolution {
    Pod(TargetPod),
    /// No pod can take a tunnel now, for this reason. A change of the stores can end it.
    NoPod(String),
    /// A store has no complete list yet, so "no pod" is not known.
    NotKnown(String),
}

/// The object of a workload rule in its store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Owner {
    Found(Arc<str>),
    Gone,
    NotLoaded,
}

/// What the resolution reads.
pub(super) struct Inputs<'a> {
    pub(super) pods: &'a Snapshot<Pod>,
    pub(super) services: &'a Snapshot<Service>,
    pub(super) owners: &'a Index<Owners>,
    /// The object of the rule, for a workload.
    pub(super) owner: Owner,
}

/// One pod that can take a tunnel.
struct Candidate {
    target: TargetPod,
    ready_since: Option<Time>,
}

/// The target of `rule`. `keep` is the pod of now, and `avoid` has the UIDs of pods that the
/// server did not find a moment ago: the store can be behind a delete.
pub(super) fn resolve(
    rule: &ForwardRule,
    inputs: &Inputs<'_>,
    keep: Option<&TargetPod>,
    avoid: &[Arc<str>],
) -> Resolution {
    if !inputs.pods.listed {
        return Resolution::NotKnown("the pods are not loaded yet".to_owned());
    }
    let found = match rule.kind {
        Kind::Pod => of_pod(rule, inputs.pods),
        Kind::Service => of_service(rule, inputs),
        _ => of_workload(rule, inputs),
    };
    let mut candidates = match found {
        Ok(candidates) => candidates,
        Err(resolution) => return resolution,
    };
    candidates.retain(|candidate| !avoid.contains(&candidate.target.uid));
    let any_ready = candidates.iter().any(|candidate| candidate.target.ready);
    // The pod of now is the same pod and port, also after a change of its readiness.
    let same = |candidate: &Candidate| {
        keep.is_some_and(|keep| (&keep.uid, keep.port) == (&candidate.target.uid, candidate.target.port))
    };
    let fits = |candidate: &Candidate| same(candidate) && (candidate.target.ready || !any_ready);
    if let Some(kept) = candidates.iter().find(|candidate| fits(candidate)) {
        return Resolution::Pod(kept.target.clone());
    }
    // A ready pod first, then the one that is ready for the longest time, as kubectl.
    candidates.sort_by(|a, b| {
        let key = |candidate: &Candidate| (!candidate.target.ready, candidate.ready_since.clone());
        key(a).cmp(&key(b)).then_with(|| a.target.pod.cmp(&b.target.pod))
    });
    match candidates.into_iter().next() {
        Some(candidate) => Resolution::Pod(candidate.target),
        None => Resolution::NoPod(format!("{} has no pod that runs", object(rule))),
    }
}

/// "the Service shop/web".
fn object(rule: &ForwardRule) -> String {
    format!("the {} {}/{}", rule.kind.info().name, rule.key.namespace, rule.key.name)
}

fn of_pod(rule: &ForwardRule, pods: &Snapshot<Pod>) -> Result<Vec<Candidate>, Resolution> {
    let Some(pod) = pods.items.get(&rule.key) else {
        return Err(Resolution::NoPod(format!("{} does not exist", object(rule))));
    };
    if let Some(reason) = not_running(pod) {
        return Err(Resolution::NoPod(format!("{} {reason}", object(rule))));
    }
    Ok(candidate(pod, rule.port).into_iter().collect())
}

fn of_service(rule: &ForwardRule, inputs: &Inputs<'_>) -> Result<Vec<Candidate>, Resolution> {
    if !inputs.services.listed {
        return Err(Resolution::NotKnown("the Services are not loaded yet".to_owned()));
    }
    let Some(service) = inputs.services.items.get(&rule.key) else {
        return Err(Resolution::NoPod(format!("{} does not exist", object(rule))));
    };
    let ports = service.spec.as_ref().and_then(|spec| spec.ports.as_deref()).unwrap_or_default();
    let Some(port) = ports.iter().find(|port| port.port == i32::from(rule.port)) else {
        return Err(Resolution::NoPod(format!("{} has no port {}", object(rule), rule.port)));
    };
    if let Some(protocol) = port.protocol.as_deref().filter(|protocol| *protocol != "TCP") {
        let text = format!("port {} of {} is {protocol}, and a forward carries TCP", rule.port, object(rule));
        return Err(Resolution::NoPod(text));
    }
    let answer = inputs.owners.pods_of_service(&rule.key);
    if !answer.basis.is_loaded() {
        return Err(Resolution::NotKnown("the EndpointSlices are not loaded yet".to_owned()));
    }
    if answer.found.is_empty() {
        return Err(Resolution::NoPod(format!("{} has no pod in its EndpointSlices", object(rule))));
    }
    let mut candidates = Vec::new();
    for endpoint in &answer.found {
        let Some(pod) = inputs.pods.items.get(&endpoint.pod).filter(|pod| not_running(pod).is_none()) else {
            continue;
        };
        if let Some(mut found) = target_port(port, pod).and_then(|number| candidate(pod, number)) {
            found.target.ready &= endpoint.ready;
            candidates.push(found);
        }
    }
    Ok(candidates)
}

/// The port in the pod of a port of a Service: a number, or the name of a container port.
/// With no target port, the pod has the port of the Service.
fn target_port(port: &ServicePort, pod: &Pod) -> Option<u16> {
    match &port.target_port {
        None => u16::try_from(port.port).ok(),
        Some(IntOrString::Int(number)) => u16::try_from(*number).ok(),
        Some(IntOrString::String(name)) => {
            let containers = pod.spec.as_ref().map(|spec| spec.containers.as_slice()).unwrap_or_default();
            let mut ports = containers.iter().flat_map(|container| container.ports.iter().flatten());
            let named = ports.find(|candidate| candidate.name.as_deref() == Some(name))?;
            u16::try_from(named.container_port).ok()
        }
    }
}

fn of_workload(rule: &ForwardRule, inputs: &Inputs<'_>) -> Result<Vec<Candidate>, Resolution> {
    let uid = match &inputs.owner {
        Owner::Found(uid) => uid,
        Owner::Gone => return Err(Resolution::NoPod(format!("{} does not exist", object(rule)))),
        Owner::NotLoaded => {
            let text = format!("the {} are not loaded yet", rule.kind.info().title);
            return Err(Resolution::NotKnown(text));
        }
    };
    let answer = inputs.owners.pods_of(rule.kind, uid);
    if !answer.basis.is_loaded() {
        return Err(Resolution::NotKnown("the owners of the pods are not loaded yet".to_owned()));
    }
    let in_store = |owned: &crate::index::ObjectRef| {
        let pod = inputs.pods.items.get(&owned.key)?;
        (pod.metadata.uid.as_deref() == Some(&*owned.uid) && not_running(pod).is_none()).then_some(pod)
    };
    Ok(answer.found.iter().filter_map(in_store).filter_map(|pod| candidate(pod, rule.port)).collect())
}

/// Why a pod cannot take a tunnel: it does not run, or it ends. `None` for a pod that runs.
fn not_running(pod: &Pod) -> Option<String> {
    if pod.metadata.deletion_timestamp.is_some() {
        return Some("ends".to_owned());
    }
    let phase = pod.status.as_ref().and_then(|status| status.phase.as_deref()).unwrap_or("Unknown");
    (phase != "Running").then(|| format!("does not run: its phase is {phase}"))
}

fn candidate(pod: &Pod, port: u16) -> Option<Candidate> {
    let conditions = pod.status.as_ref().and_then(|status| status.conditions.as_deref()).unwrap_or_default();
    let ready = conditions.iter().find(|condition| condition.type_ == "Ready" && condition.status == "True");
    let target = TargetPod {
        pod: ObjectKey::of(&pod.metadata)?,
        uid: pod.metadata.uid.as_deref()?.into(),
        port,
        ready: ready.is_some(),
    };
    Some(Candidate { target, ready_since: ready.and_then(|condition| condition.last_transition_time.clone()) })
}

#[cfg(test)]
mod tests;
