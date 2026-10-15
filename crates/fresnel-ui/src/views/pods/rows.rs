//! The rows of the Pod table: which pods pass the filters, their sequence and their cells.
//!
//! The columns Ready, Status, Restarts and Last restart have the values of
//! `kubectl get pods` (plan section 11.2), CPU and Memory those of the last poll of the
//! metrics (`views/metrics`). A sort value comes from the pod and the poll alone: the age
//! sorts by the creation time, so no row moves because time passes.

use std::cmp::{Ordering, Reverse};
use std::sync::Arc;
use std::time::SystemTime;

use fresnel_core::cluster::ClusterId;
use fresnel_core::kinds::{Kind, ObjectHealth, PodStatus};
use fresnel_core::store::ObjectKey;
use k8s_openapi::api::core::v1::Pod;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;

use crate::format::{self, Clock};
use crate::views::metrics;
use crate::views::metrics::gauge::{Resource, Usage};
use crate::views::objects::{NamespaceSet, Namespaces, Nouns, ObjectFilter, ObjectModel, RowScene, Stores, uid};
use crate::widgets::table::model::{Cell, Column, Computed, Lead, Model, ObjectId, contains_ignore_case};
use crate::widgets::tone::Tone;
use crate::widgets::worker::Cancel;

/// The columns of the table, in their sequence.
#[derive(Clone, Copy)]
enum Field {
    Name,
    Namespace,
    Cluster,
    Ready,
    Status,
    Restarts,
    LastRestart,
    Node,
    Age,
    Usage(Resource),
}

const FIELDS: [Field; 11] = [
    Field::Name,
    Field::Namespace,
    Field::Cluster,
    Field::Ready,
    Field::Status,
    Field::Restarts,
    Field::LastRestart,
    Field::Node,
    Field::Age,
    Field::Usage(Resource::Cpu),
    Field::Usage(Resource::Memory),
];

const COLUMNS: [Column; 11] = [
    Column::new("Name", 220.0),
    Column::new("Namespace", 120.0),
    Column::new("Cluster", 120.0),
    Column::new("Ready", 56.0).numeric(),
    Column::new("Status", 140.0),
    Column::new("Restarts", 70.0).numeric(),
    Column::new("Last restart", 100.0),
    Column::new("Node", 130.0),
    Column::new("Age", 84.0).numeric(),
    Resource::Cpu.column(),
    Resource::Memory.column(),
];

/// The text of a cell with no value.
const NONE: &str = "–";
/// The count of pods between two reads of the cancel mark.
const CANCEL_STEP: usize = 1024;

/// One row: a pod with its identity, the state that `kubectl get pods` prints and its
/// metrics.
pub(crate) struct PodRow {
    pub(crate) id: ObjectId,
    pub(crate) pod: Arc<Pod>,
    pub(crate) status: PodStatus,
    pub(crate) usage: Option<Usage>,
}

impl PodRow {
    fn node(&self) -> Option<&str> {
        self.pod.spec.as_ref()?.node_name.as_deref()
    }

    fn created(&self) -> Option<&Time> {
        self.pod.metadata.creation_timestamp.as_ref()
    }
}

/// The age of a time of the cluster on the clock of the view.
pub(crate) fn age(clock: &Clock, time: &Time) -> String {
    format::span(clock.since(SystemTime::from(time.0)))
}

pub(crate) struct PodModel;

impl PodModel {
    fn matches(cluster: &ClusterId, key: &ObjectKey, pod: &Pod, status: &PodStatus, needle: &str) -> bool {
        let node = pod.spec.as_ref().and_then(|spec| spec.node_name.as_deref()).unwrap_or_default();
        [&*key.name, &*key.namespace, cluster.as_str(), &status.text, node]
            .iter()
            .any(|text| contains_ignore_case(text, needle))
    }
}

impl Model for PodModel {
    type Data = Stores<Pod>;
    type Filter = ObjectFilter;
    type Row = PodRow;
    type Summary = Namespaces;
    type Scene = RowScene;

    fn columns(&self) -> &[Column] {
        &COLUMNS
    }

    fn rows(
        &self,
        data: &Stores<Pod>,
        filter: &ObjectFilter,
        cancel: &Cancel<'_>,
    ) -> Option<Computed<PodRow, Namespaces>> {
        let needle = filter.needle();
        let (mut rows, mut total, mut namespaces) = (Vec::new(), 0, NamespaceSet::default());
        for (cluster, snapshot) in &data.clusters {
            total += snapshot.items.len();
            let metered = data.metered(cluster);
            for (count, (key, pod)) in snapshot.items.iter().enumerate() {
                if count.is_multiple_of(CANCEL_STEP) && cancel.is_set() {
                    return None;
                }
                namespaces.add(key);
                // A pod of an other namespace needs no status.
                if !(filter.namespaces.is_empty() || filter.namespaces.contains(&key.namespace)) {
                    continue;
                }
                let status = PodStatus::of(pod);
                if !filter.passes(key, status.health) || !Self::matches(cluster, key, pod, &status, &needle) {
                    continue;
                }
                let id = ObjectId { cluster: cluster.clone(), key: key.clone(), uid: uid(&pod.metadata).into() };
                let usage = metered.and_then(|metered| metered.usage(key, &**pod));
                rows.push(PodRow { id, pod: Arc::clone(pod), status, usage });
            }
        }
        Some(Computed { rows, total, summary: namespaces.done() })
    }

    fn compare(&self, a: &PodRow, b: &PodRow, column: usize) -> Ordering {
        match FIELDS.get(column) {
            Some(Field::Name) => a.id.key.name.cmp(&b.id.key.name),
            Some(Field::Namespace) => a.id.key.namespace.cmp(&b.id.key.namespace),
            Some(Field::Cluster) => a.id.cluster.cmp(&b.id.cluster),
            Some(Field::Ready) => (a.status.ready, a.status.containers).cmp(&(b.status.ready, b.status.containers)),
            Some(Field::Status) => a.status.text.cmp(&b.status.text),
            Some(Field::Restarts) => a.status.restarts.cmp(&b.status.restarts),
            Some(Field::LastRestart) => a.status.last_restart.cmp(&b.status.last_restart),
            Some(Field::Node) => a.node().cmp(&b.node()),
            Some(Field::Usage(resource)) => metrics::cells::compare(a.usage.as_ref(), b.usage.as_ref(), *resource),
            // A small age is a late creation time.
            Some(Field::Age) => Reverse(a.created()).cmp(&Reverse(b.created())),
            None => Ordering::Equal,
        }
    }

    fn tie(&self, a: &PodRow, b: &PodRow) -> Ordering {
        a.id.key.cmp(&b.id.key).then_with(|| a.id.cluster.cmp(&b.id.cluster))
    }

    fn id<'a>(&self, row: &'a PodRow) -> &'a ObjectId {
        &row.id
    }

    fn has(&self, data: &Stores<Pod>, row: &PodRow) -> bool {
        data.has(&row.id)
    }

    fn cell<'a>(&self, row: &'a PodRow, column: usize, scene: &'a RowScene) -> Cell<'a> {
        let old = scene.old(&row.id.cluster);
        let status = &row.status;
        let cell = match FIELDS.get(column) {
            Some(Field::Name) => Cell::new(&*row.id.key.name).lead(old.map_or(Lead::None, Lead::Old)),
            Some(Field::Namespace) => Cell::new(&*row.id.key.namespace).tone(Tone::Muted),
            Some(Field::Cluster) => Cell::new(row.id.cluster.as_str()).lead(Lead::Dot(scene.color(&row.id.cluster))),
            Some(Field::Ready) => {
                let short = status.ready < status.containers && status.health != ObjectHealth::Healthy;
                let tone = if short { Tone::Warn } else { Tone::Normal };
                Cell::new(format!("{}/{}", status.ready, status.containers)).tone(tone)
            }
            Some(Field::Status) => {
                let tone = if old.is_some() { Tone::Faint } else { Tone::of_object(status.health) };
                Cell::new(status.text.as_str()).tone(tone).lead(Lead::Dot(tone.color(scene.palette)))
            }
            Some(Field::Restarts) => {
                let tone = if status.restarts == 0 { Tone::Muted } else { Tone::Normal };
                Cell::new(status.restarts.to_string()).tone(tone)
            }
            Some(Field::LastRestart) => match &status.last_restart {
                Some(time) => Cell::new(format!("{} ago", age(&scene.clock, time))).tone(Tone::Muted),
                None => Cell::new(NONE).tone(Tone::Faint),
            },
            Some(Field::Node) => Cell::new(row.node().unwrap_or(NONE)).tone(Tone::Muted),
            Some(Field::Usage(resource)) => {
                let problem = scene.problem(&row.id.cluster, Kind::Pod);
                metrics::cells::cell(row.usage.as_ref(), *resource, problem, &scene.clock)
            }
            Some(Field::Age) => {
                let text = row.created().map_or_else(|| NONE.to_owned(), |time| age(&scene.clock, time));
                Cell::new(text).tone(Tone::Muted)
            }
            None => Cell::new(""),
        };
        // An old row has no color that tells a state of now.
        if old.is_some() { cell.faint() } else { cell }
    }
}

impl ObjectModel for PodModel {
    type Object = Pod;

    fn nouns(&self) -> Nouns {
        Nouns { title: "Pods", one: "pod", many: "pods" }
    }

    fn object<'a>(&self, row: &'a PodRow) -> &'a Arc<Pod> {
        &row.pod
    }
}

#[cfg(test)]
mod tests;
