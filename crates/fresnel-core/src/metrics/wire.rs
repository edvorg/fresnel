//! The answers of `metrics.k8s.io/v1beta1`: own types that accept a missing field.
//!
//! `k8s-metrics` 0.28.0 fails the full list when one container has no `memory` (research 11
//! section 7.2). Here an item that does not fit is one unread item with its reason, and the
//! other items stay. An item with no value of CPU or memory has no sample: a value that is
//! not there is not zero.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use k8s_openapi::jiff::Timestamp;
use serde::Deserialize;
use serde_json::Value;

use super::resources::Amount;
use super::{duration, quantity};
use crate::store::ObjectKey;

/// The path of the PodMetrics of each namespace.
pub(crate) const PODS: &str = "/apis/metrics.k8s.io/v1beta1/pods";
/// The path of the NodeMetrics.
pub(crate) const NODES: &str = "/apis/metrics.k8s.io/v1beta1/nodes";

/// One list of the API with each item as JSON, so that a bad item is not a bad list.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct RawList {
    #[serde(default)]
    pub(crate) items: Vec<Value>,
}

/// The fields of a PodMetrics item and of a NodeMetrics item.
#[derive(Default, Deserialize)]
#[serde(default)]
struct Item {
    metadata: Meta,
    timestamp: Option<String>,
    window: Option<String>,
    /// PodMetrics only.
    containers: Vec<Container>,
    /// NodeMetrics only.
    usage: BTreeMap<String, String>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Meta {
    name: Option<String>,
    namespace: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Container {
    name: Option<String>,
    usage: BTreeMap<String, String>,
}

/// The usage of one pod, of each of its containers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PodSample {
    pub(crate) key: ObjectKey,
    /// The time of the sample on the clock of the cluster, in seconds since 1970.
    pub(crate) at: u32,
    pub(crate) window: Option<Duration>,
    pub(crate) containers: Vec<(Arc<str>, Amount)>,
}

impl PodSample {
    /// The sum of the containers.
    pub(crate) fn usage(&self) -> Amount {
        self.containers.iter().map(|(_, usage)| *usage).sum()
    }
}

/// The usage of one node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NodeSample {
    pub(crate) name: Arc<str>,
    pub(crate) at: u32,
    pub(crate) window: Option<Duration>,
    pub(crate) usage: Amount,
}

/// The items of one answer that the sampler could read, and the others.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Read<T> {
    pub(crate) items: Vec<T>,
    /// The count of items that the sampler could not read.
    pub(crate) unread: u64,
    /// Why the first of them could not be read.
    pub(crate) problem: Option<Arc<str>>,
}

/// The PodMetrics of a list.
pub(crate) fn pods(list: RawList) -> Read<PodSample> {
    let mut keys = BTreeSet::new();
    read(list, |item| {
        let key = ObjectKey::new(item.metadata.namespace.as_deref().unwrap_or_default(), &named(&item.metadata)?);
        let at = at(&item)?;
        if key.namespace.is_empty() || item.containers.is_empty() {
            return Err(format!("the pod {} has no namespace or no container", key.name));
        }
        let usage = |container: &Container| {
            let name = container.name.as_deref().filter(|name| !name.is_empty());
            let name = name.ok_or_else(|| format!("a container of the pod {} has no name", key.name))?;
            let usage =
                amount(&container.usage).map_err(|why| format!("the container {name} of {}: {why}", key.name))?;
            Ok::<_, String>((Arc::from(name), usage))
        };
        let containers = item.containers.iter().map(usage).collect::<Result<_, _>>()?;
        once(&mut keys, key.clone(), || format!("the pod {}", key.name))?;
        Ok(PodSample { key, at, window: window(&item), containers })
    })
}

/// The NodeMetrics of a list.
pub(crate) fn nodes(list: RawList) -> Read<NodeSample> {
    let mut names = BTreeSet::new();
    read(list, |item| {
        let name = named(&item.metadata)?;
        let usage = amount(&item.usage).map_err(|why| format!("the node {name}: {why}"))?;
        let at = at(&item)?;
        once(&mut names, name.clone(), || format!("the node {name}"))?;
        Ok(NodeSample { at, window: window(&item), usage, name: name.into() })
    })
}

/// Reads each item with `one`. An item that is no JSON object of the API is unread too.
fn read<T>(list: RawList, mut one: impl FnMut(Item) -> Result<T, String>) -> Read<T> {
    let mut read = Read { items: Vec::with_capacity(list.items.len()), unread: 0, problem: None };
    for value in list.items {
        // The text of the JSON reader can hold a value of the item, so the reason has none.
        let item = serde_json::from_value(value).map_err(|_| "an item does not fit the API".to_owned());
        match item.and_then(&mut one) {
            Ok(sample) => read.items.push(sample),
            Err(why) => {
                read.unread += 1;
                read.problem.get_or_insert_with(|| why.into());
            }
        }
    }
    read
}

/// An error for a second item of an object in one list: the sums would count it twice. The
/// first item stays, because no rule tells which of the two is right.
fn once<K: Ord>(keys: &mut BTreeSet<K>, key: K, object: impl FnOnce() -> String) -> Result<(), String> {
    if keys.insert(key) { Ok(()) } else { Err(format!("{} comes twice", object())) }
}

fn named(meta: &Meta) -> Result<String, String> {
    meta.name.clone().filter(|name| !name.is_empty()).ok_or_else(|| "an item has no name".to_owned())
}

/// The time of the sample: the end of its window, on the clock of the cluster.
fn at(item: &Item) -> Result<u32, String> {
    let text = item.timestamp.as_deref().unwrap_or_default();
    let time: Timestamp = text.parse().map_err(|_| "the time of an item is not RFC 3339".to_owned())?;
    u32::try_from(time.as_second()).map_err(|_| "the time of an item is before 1970 or after 2105".to_owned())
}

/// The window of the sample. It only tells the resolution of the server, so a window that
/// is not a duration of Go costs the sample nothing.
fn window(item: &Item) -> Option<Duration> {
    item.window.as_deref().and_then(|text| duration::parse(text).ok())
}

/// The CPU and the memory of a map of usage. Each one must be there and readable.
fn amount(usage: &BTreeMap<String, String>) -> Result<Amount, String> {
    let value = |name: &str, read: fn(&str) -> Result<u64, &'static str>| {
        let text = usage.get(name).ok_or_else(|| format!("no {name}"))?;
        read(text).map_err(|why| format!("the {name}: {why}"))
    };
    Ok(Amount { cpu: value("cpu", quantity::nanocores)?, memory: value("memory", quantity::bytes)? })
}

#[cfg(test)]
mod tests;
