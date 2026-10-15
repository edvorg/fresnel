//! The lines of the charts of the Metrics tab (research 11 section 8.2): a line for each
//! container of a pod and one for the pod; the sum of a workload and its largest pods; the
//! usage of a node with its system processes. The request and the limit, or the requests,
//! the allocatable part and the capacity of a node, are fixed lines.
//!
//! A line has a gap where samples are missing: more than 2.5 times the distance of its tier
//! (`Series::fine_from`). A poll that gets the same scrape of the server again misses one
//! sample; that is no gap.

use std::any::Any;

use egui::Color32;
use fresnel_core::kinds::{Describe, Kind, Object};
use fresnel_core::metrics::{Series, SeriesKey};
use k8s_openapi::api::core::v1::Node;

use super::gauge::{Resource, Usage, Whole};
use super::read::Metered;
use crate::theme::Palette;
use crate::views::objects::uid;
use crate::widgets::chart::{Line, MOST_LINES, Mark};
use crate::widgets::table::model::ObjectId;

/// The distance of two samples, in expected distances, from which a line has a gap.
pub(super) const GAP: f64 = 2.5;

/// The lines of one resource of an object.
pub(crate) struct Lines {
    pub(crate) lines: Vec<Line>,
    pub(crate) marks: Vec<Mark>,
    /// The pods of a workload with no line of their own: the line of the sum has them.
    pub(crate) more: usize,
}

/// The series of an object: the lines that it always has, and the pods of a workload, of
/// which the largest get a line.
struct Named<'a> {
    always: Vec<(String, &'a Series)>,
    pods: Vec<(String, &'a Series)>,
}

/// The lines of each resource of an object with the samples from `from`, in seconds on the
/// clock of the cluster.
pub(crate) fn of<K: Describe>(
    metered: &Metered,
    id: &ObjectId,
    object: &K,
    from: u32,
    palette: &Palette,
) -> [Lines; 2] {
    let named = named(metered, id, object);
    let usage = metered.usage(&id.key, object);
    let node: Option<&Node> = (object as &dyn Any).downcast_ref();
    let period = u32::try_from(metered.metrics.period.as_secs()).unwrap_or(u32::MAX);
    Resource::BOTH.map(|resource| {
        let mut pods: Vec<&(String, &Series)> = named.pods.iter().collect();
        let latest = |series: &Series| series.latest().map(|sample| resource.of(sample.usage));
        pods.sort_by_key(|(name, series)| (std::cmp::Reverse(latest(series)), name.clone()));
        let room = MOST_LINES.saturating_sub(named.always.len());
        let shown = named.always.iter().chain(pods.iter().copied().take(room));
        let lines = shown.enumerate().map(|(number, (name, series))| {
            let color = if number == 0 { palette.text } else { color(palette, number - 1) };
            Line::new(name.clone(), color, points(series, from, period, resource))
        });
        let marks = marks(usage.as_ref(), node, resource, palette);
        Lines { lines: lines.collect(), marks, more: pods.len().saturating_sub(room) }
    })
}

/// The series of the lines of an object, by its kind.
fn named<'a, K: Describe>(metered: &'a Metered, id: &'a ObjectId, object: &K) -> Named<'a> {
    let (metrics, cluster) = (&metered.metrics, &id.cluster);
    let mut named = Named { always: Vec::new(), pods: Vec::new() };
    let mut add = |name: &str, key: SeriesKey| {
        named.always.extend(metrics.series(cluster, &key).map(|series| (name.to_owned(), series)))
    };
    match K::ID {
        Kind::Pod => {
            let containers: Vec<(&str, &Series)> = metrics.containers(cluster, &id.key).collect();
            if containers.len() != 1 {
                add("pod", SeriesKey::Pod(id.key.clone()));
            }
            named.always.extend(containers.into_iter().map(|(name, series)| (name.to_owned(), series)));
        }
        Kind::Node => add("node", SeriesKey::Node(id.key.name.clone())),
        kind => {
            add("all pods", SeriesKey::Workload(kind, id.key.clone()));
            let owned = metered.owners.pods_of(kind, uid(object.meta())).found;
            let series = |key| metrics.series(cluster, &SeriesKey::Pod(key));
            named.pods =
                owned.into_iter().filter_map(|pod| Some((pod.key.name.to_string(), series(pod.key)?))).collect();
        }
    }
    named
}

/// The points of a series from `from`, with a gap where samples are missing.
fn points(series: &Series, from: u32, period: u32, resource: Resource) -> Vec<Option<[f64; 2]>> {
    let (fine_from, mut last, mut points) = (series.fine_from(), None::<u32>, Vec::new());
    for sample in series.samples().filter(|sample| sample.at >= from) {
        if let Some(last) = last {
            let coarse = fine_from.is_none_or(|first| last < first);
            let expected = if coarse { series.step() } else { period };
            if f64::from(sample.at.saturating_sub(last)) > GAP * f64::from(expected.max(1)) {
                points.push(None);
            }
        }
        last = Some(sample.at);
        points.push(Some([f64::from(sample.at), resource.value(resource.of(sample.usage))]));
    }
    points
}

/// The fixed lines of one resource: the request and the limit of the last sample, or the
/// requests of the pods on a node, its allocatable part and its capacity.
fn marks(usage: Option<&Usage>, node: Option<&Node>, resource: Resource, palette: &Palette) -> Vec<Mark> {
    let mark = |name: &str, amount: u64, color| Mark { name: name.to_owned(), value: resource.value(amount), color };
    let mut marks = Vec::new();
    if let Some(usage) = usage {
        let gauge = usage.of(resource);
        let (request, most) = match usage.whole {
            Whole::Limit => ("request", "limit"),
            Whole::Allocatable => ("requests of its pods", "allocatable"),
        };
        marks.extend(gauge.request.map(|amount| mark(request, amount, palette.text_muted)));
        marks.extend(gauge.most.map(|amount| mark(most, amount, palette.warn)));
    }
    let capacity = resource.read(node.and_then(|node| node.status.as_ref()?.capacity.as_ref()));
    marks.extend(capacity.map(|amount| mark("capacity", amount, palette.text_faint)));
    marks
}

/// The color of a line after the first one.
fn color(palette: &Palette, number: usize) -> Color32 {
    palette.clusters.get(number % palette.clusters.len()).copied().unwrap_or(palette.accent)
}
