//! The samples of one answer for each series: the containers and pods as the server gives
//! them, and the sums of the workloads, namespaces and the cluster (research 11 section 6).
//!
//! A sum has only the objects with a value, and its cover tells how many it expected: a
//! missing value is not zero. A sum with no value at all has no sample.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::history::SeriesKey;
use super::series::{Cover, Sample};
use super::wire::{NodeSample, PodSample};
use super::world::World;
use crate::store::ObjectKey;

/// The samples of each container, pod, workload and namespace of an answer of PodMetrics.
/// A workload is the sum of the pods that the owner index gives it at this poll.
pub(crate) fn of_pods(pods: &[PodSample], world: &World) -> Vec<(SeriesKey, Sample)> {
    let mut samples = Vec::new();
    for pod in pods {
        for (name, usage) in &pod.containers {
            let key = SeriesKey::Container { pod: pod.key.clone(), name: Arc::clone(name) };
            samples.push((key, Sample { at: pod.at, cover: Cover::ONE, usage: *usage }));
        }
        samples.push((SeriesKey::Pod(pod.key.clone()), Sample { at: pod.at, cover: Cover::ONE, usage: pod.usage() }));
    }
    let by_key: BTreeMap<&ObjectKey, &PodSample> = pods.iter().map(|pod| (&pod.key, pod)).collect();
    for (kind, workloads) in &world.workloads {
        for (key, owned) in &workloads.items {
            let sampled: Vec<&PodSample> = owned.iter().filter_map(|pod| by_key.get(pod).copied()).collect();
            let expected = owned.iter().filter(|pod| world.runs(pod)).count();
            if let Some(sample) = sum(&sampled, expected) {
                samples.push((SeriesKey::Workload(*kind, key.clone()), sample));
            }
        }
    }
    let mut namespaces: BTreeMap<&str, Vec<&PodSample>> = BTreeMap::new();
    for pod in pods {
        namespaces.entry(&pod.key.namespace).or_default().push(pod);
    }
    let mut running: BTreeMap<&str, usize> = BTreeMap::new();
    for (key, _) in world.pods.items.iter().filter(|(_, runs)| **runs) {
        *running.entry(&key.namespace).or_default() += 1;
    }
    for (namespace, sampled) in namespaces {
        let expected = running.get(namespace).copied().unwrap_or_default();
        if let Some(sample) = sum(&sampled, expected) {
            samples.push((SeriesKey::Namespace(namespace.into()), sample));
        }
    }
    samples
}

/// The samples of each node of an answer of NodeMetrics, and of the cluster: the sum of
/// the nodes, which counts the system processes too.
pub(crate) fn of_nodes(nodes: &[NodeSample], world: &World) -> Vec<(SeriesKey, Sample)> {
    let mut samples: Vec<_> = nodes
        .iter()
        .map(|node| {
            (SeriesKey::Node(Arc::clone(&node.name)), Sample { at: node.at, cover: Cover::ONE, usage: node.usage })
        })
        .collect();
    let at = nodes.iter().map(|node| node.at).max();
    if let Some(at) = at {
        let cover = Cover::new(nodes.len(), world.nodes.items.len());
        samples.push((SeriesKey::Cluster, Sample { at, cover, usage: nodes.iter().map(|node| node.usage).sum() }));
    }
    samples
}

/// The sum of the pods with a value, at the latest time of their samples. `None` with no pod.
fn sum(sampled: &[&PodSample], expected: usize) -> Option<Sample> {
    let at = sampled.iter().map(|pod| pod.at).max()?;
    let usage = sampled.iter().map(|pod| pod.usage()).sum();
    Some(Sample { at, cover: Cover::new(sampled.len(), expected), usage })
}

#[cfg(test)]
mod tests;
