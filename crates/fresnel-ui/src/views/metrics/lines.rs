//! The lines of the charts of the Metrics tab (research 11 section 8.2): the line of the
//! whole object, and a line for each of its largest parts: the containers of a pod, the pods
//! of a workload. A node has the line of its usage with its system processes. The request
//! and the limit, or the requests, the allocatable part and the capacity of a node, are
//! fixed lines.
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
use crate::format;
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
    /// The parts with no line of their own in words, when there are such parts: the line
    /// of the whole has them.
    pub(crate) more: Option<String>,
}

/// The series of an object: the line of the whole, and its parts, of which the largest get
/// a line of their own (`MOST_LINES`): a pod can have many containers, a workload many pods.
struct Named<'a> {
    whole: Option<(&'static str, &'a Series)>,
    parts: Vec<(String, &'a Series)>,
    /// The noun of a part, for the note of the parts with no line. A node has no parts.
    noun: &'static str,
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
    let room = MOST_LINES - usize::from(named.whole.is_some());
    Resource::BOTH.map(|resource| {
        let latest = |series: &Series| series.latest().map(|sample| resource.of(sample.usage));
        let mut parts: Vec<&(String, &Series)> = named.parts.iter().collect();
        // The largest parts get a line. The lines then keep the sequence of their names: the
        // color of a line is of its place, and it must not change when two values change places.
        parts.sort_by_key(|(_, series)| std::cmp::Reverse(latest(series)));
        parts.truncate(room);
        parts.sort_by(|(a, _), (b, _)| a.cmp(b));
        let shown = named.whole.into_iter().chain(parts.iter().map(|(name, series)| (name.as_str(), *series)));
        let lines = shown.enumerate().map(|(number, (name, series))| {
            let color = if number == 0 { palette.text } else { color(palette, number - 1) };
            Line::new(name.to_owned(), color, points(series, from, period, resource))
                .with_legend(legend(name, &id.key.name))
        });
        let hidden = named.parts.len() - parts.len();
        let more = (hidden > 0).then(|| {
            let hidden = format::count(hidden as u64, named.noun);
            named.whole.map_or_else(
                || format!("{hidden} with no line."),
                |(whole, _)| format!("{hidden} in the line \"{whole}\"."),
            )
        });
        Lines { lines: lines.collect(), marks: marks(usage.as_ref(), node, resource, palette), more }
    })
}

/// The series of the lines of an object, by its kind. A pod with one container has the line
/// of the container only: the line of the pod would be the same line.
fn named<'a, K: Describe>(metered: &'a Metered, id: &'a ObjectId, object: &K) -> Named<'a> {
    let (metrics, cluster) = (&metered.metrics, &id.cluster);
    let series = |key: SeriesKey| metrics.series(cluster, &key);
    match K::ID {
        Kind::Pod => {
            let parts: Vec<(String, &Series)> =
                metrics.containers(cluster, &id.key).map(|(name, series)| (name.to_owned(), series)).collect();
            let whole = (parts.len() != 1).then(|| series(SeriesKey::Pod(id.key.clone()))).flatten();
            Named { whole: whole.map(|series| ("pod", series)), parts, noun: "more container" }
        }
        Kind::Node => {
            let whole = series(SeriesKey::Node(id.key.name.clone())).map(|series| ("node", series));
            Named { whole, parts: Vec::new(), noun: "" }
        }
        kind => {
            let whole = series(SeriesKey::Workload(kind, id.key.clone())).map(|series| ("all pods", series));
            let owned = metered.owners.pods_of(kind, uid(object.meta())).found;
            let parts =
                owned.into_iter().filter_map(|pod| Some((pod.key.name.to_string(), series(SeriesKey::Pod(pod.key))?)));
            Named { whole, parts: parts.collect(), noun: "more pod" }
        }
    }
}

/// The name of a line in the legend: the random end of the name of a pod, as `gn8jp` for
/// `checkout-api-7d9c5b8f6-gn8jp`. The title of the panel names the workload, and the end
/// tells its pods apart; six full names made the legend cover the newest half of a chart.
/// The number of a pod of a StatefulSet alone would read as a value, so it keeps its name.
fn legend<'a>(name: &'a str, workload: &str) -> &'a str {
    let own = name.strip_prefix(workload).and_then(|rest| rest.strip_prefix('-'));
    let end = own.and_then(|rest| rest.rsplit('-').next());
    end.filter(|end| !end.is_empty() && !end.bytes().all(|byte| byte.is_ascii_digit())).unwrap_or(name)
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

#[cfg(test)]
mod tests {
    use super::legend;

    #[test]
    fn the_legend_names_a_pod_by_the_end_of_its_name() {
        assert_eq!(legend("checkout-api-7d9c5b8f6-gn8jp", "checkout-api"), "gn8jp");
        assert_eq!(legend("node-exporter-x7k2p", "node-exporter"), "x7k2p");
        assert_eq!(legend("report-29999640-k2x7p", "report"), "k2x7p", "a pod of a Job of a CronJob");
        assert_eq!(legend("postgres-1", "postgres"), "postgres-1", "a number alone reads as a value");
        assert_eq!(legend("all pods", "web"), "all pods");
        assert_eq!(legend("web", "web"), "web");
        assert_eq!(legend("webhook-1a", "web"), "webhook-1a", "an other name with the same start");
    }
}
