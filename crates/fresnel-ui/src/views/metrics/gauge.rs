//! The usage of one object at one sample, as a cell of a table shows it (research 11
//! section 8.1): the usage of each resource against its request and its limit, or against
//! the allocatable part of a node, with the rules of the bar, its color and its words.

use std::collections::BTreeMap;
use std::time::SystemTime;

use fresnel_core::metrics::{Amount, Cover, quantity};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;

use crate::format::{self, Clock};
use crate::widgets::table::model::Column;
use crate::widgets::tone::Tone;

/// The part of the limit from which a value shows as high, and from which as full
/// (research 11 section 8.1).
const HIGH: f64 = 0.8;
const FULL: f64 = 0.95;
/// The unit of the axis of a chart: millicores, and MiB, so its marks are round numbers.
const MILLICORE: f64 = 1e6;
const MIB: f64 = 1024.0 * 1024.0;

/// The reader of a quantity of one resource.
type Reader = fn(&str) -> Result<u64, &'static str>;

/// One of the two resources of the metrics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Resource {
    Cpu,
    Memory,
}

impl Resource {
    pub(crate) const BOTH: [Self; 2] = [Self::Cpu, Self::Memory];

    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Cpu => "CPU",
            Self::Memory => "Memory",
        }
    }

    /// The column of a table: the bar and the text of the largest value fit.
    pub(crate) const fn column(self) -> Column {
        match self {
            Self::Cpu => Column::new("CPU", 110.0),
            Self::Memory => Column::new("Memory", 120.0),
        }
    }

    /// The amount of this resource: nanocores or bytes.
    pub(crate) fn of(self, amount: Amount) -> u64 {
        match self {
            Self::Cpu => amount.cpu,
            Self::Memory => amount.memory,
        }
    }

    /// "250m" for CPU, "128Mi" for memory.
    pub(crate) fn text(self, amount: u64) -> String {
        match self {
            Self::Cpu => format::cpu(amount),
            Self::Memory => format::memory(amount),
        }
    }

    /// The amount of this resource in a map of quantities, as the status of a node gives
    /// its allocatable part and its capacity. `None` for a value that is not there.
    pub(crate) fn read(self, values: Option<&BTreeMap<String, Quantity>>) -> Option<u64> {
        let (name, read): (&str, Reader) = match self {
            Self::Cpu => ("cpu", quantity::nanocores),
            Self::Memory => ("memory", quantity::bytes),
        };
        values?.get(name).and_then(|value| read(&value.0).ok())
    }

    /// An amount on the axis of a chart.
    pub(crate) fn value(self, amount: u64) -> f64 {
        amount as f64 / self.unit()
    }

    /// The text of a value of the axis of a chart. A value below zero, from the margin of
    /// the axis, is zero, and zero is "0" for each resource: "0m" next to "1" read as two
    /// units.
    pub(crate) fn text_of(self, value: f64) -> String {
        match (value.max(0.0) * self.unit()) as u64 {
            0 => "0".to_owned(),
            amount => self.text(amount),
        }
    }

    fn unit(self) -> f64 {
        match self {
            Self::Cpu => MILLICORE,
            Self::Memory => MIB,
        }
    }
}

/// What the usage of an object counts against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Whole {
    /// The limit of a pod, or the sum of the limits of the pods of a workload.
    Limit,
    /// The allocatable part of a node: what its pods can request.
    Allocatable,
}

/// One resource of an object.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Gauge {
    pub(crate) usage: u64,
    /// The request, or the requests of the pods on a node. `None` for no request.
    pub(crate) request: Option<u64>,
    /// The limit, or the allocatable part of a node. `None` for no limit: a pod can then use
    /// the whole node.
    pub(crate) most: Option<u64>,
}

impl Gauge {
    /// The part of the bar: the usage of the limit, else of the request. `None` with
    /// neither, and the cell has its text only.
    pub(crate) fn part(&self) -> Option<f64> {
        let base = self.most.or(self.request).filter(|base| *base > 0)?;
        Some(self.usage as f64 / base as f64)
    }

    /// The place of the request on a bar of the limit.
    pub(crate) fn mark(&self) -> Option<f64> {
        let (request, most) = (self.request?, self.most.filter(|most| *most > 0)?);
        Some(request as f64 / most as f64)
    }

    /// High from 80 percent of the limit, full from 95 percent. A use above the request
    /// with no limit is no fault: the pod can use the node.
    pub(crate) fn tone(&self) -> Tone {
        let of_most = self.most.filter(|most| *most > 0).map(|most| self.usage as f64 / most as f64);
        match of_most {
            Some(part) if part >= FULL => Tone::Error,
            Some(part) if part >= HIGH => Tone::Warn,
            _ => Tone::Info,
        }
    }
}

/// "500m (50%)": a value with the part of it that `part` takes.
fn against(resource: Resource, value: u64, part: u64) -> String {
    let percent = (value > 0).then(|| format!(" ({:.0}%)", part as f64 / value as f64 * 100.0));
    format!("{}{}", resource.text(value), percent.unwrap_or_default())
}

/// The usage of one object at one sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Usage {
    pub(crate) cpu: Gauge,
    pub(crate) memory: Gauge,
    pub(crate) whole: Whole,
    /// The time of the sample, on the clock of the cluster.
    pub(crate) at: SystemTime,
    /// For a sum: how many of its objects had a value.
    pub(crate) cover: Cover,
    /// False for a value that the last answer of its list did not have
    /// (`fresnel_core::metrics::Latest`).
    pub(crate) current: bool,
}

impl Usage {
    pub(crate) fn of(&self, resource: Resource) -> &Gauge {
        match resource {
            Resource::Cpu => &self.cpu,
            Resource::Memory => &self.memory,
        }
    }

    /// The values of one resource in words, one for each line, with the age of the sample
    /// (plan section 12.4): "CPU 250m", "request 500m (50%)", "limit 1 (25%)", "sample of
    /// 07:59:55, 5 s ago". The age reads the time of the sample on the clock of the cluster;
    /// the mark of an old value does not depend on the two clocks.
    pub(crate) fn hint(&self, resource: Resource, clock: &Clock) -> String {
        let gauge = self.of(resource);
        let mut lines = vec![format!("{} {}", resource.title(), resource.text(gauge.usage))];
        let line = |label: &str, value: Option<String>| {
            value.map_or_else(|| format!("no {label}"), |v| format!("{label} {v}"))
        };
        match self.whole {
            Whole::Limit => {
                lines.push(line("request", gauge.request.map(|request| against(resource, request, gauge.usage))));
                lines.push(line("limit", gauge.most.map(|limit| against(resource, limit, gauge.usage))));
            }
            // The requests of the pods on a node are a part of its allocatable part too.
            Whole::Allocatable => {
                let requests = gauge.request.map(|request| {
                    let part = gauge
                        .most
                        .map(|most| format!(" ({:.0}% of allocatable)", request as f64 / most.max(1) as f64 * 100.0));
                    format!("{}{}", resource.text(request), part.unwrap_or_default())
                });
                lines.push(line("requests of its pods", requests));
                lines.push(line("allocatable", gauge.most.map(|most| against(resource, most, gauge.usage))));
            }
        }
        let age = format::span(clock.since(self.at));
        lines.push(format!("sample of {}, {age} ago", clock.time_of_day(self.at)));
        let Cover { sampled, of } = self.cover;
        if of > 1 || sampled < of {
            // A node is one sample; the sum against an allocatable part is of a cluster.
            let parts = if self.whole == Whole::Allocatable { "nodes" } else { "pods" };
            lines.push(format!("sum of {sampled} of {of} {parts}"));
        }
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    const MI: u64 = 1024 * 1024;

    fn gauge(usage: u64, request: Option<u64>, most: Option<u64>) -> Gauge {
        Gauge { usage, request, most }
    }

    #[test]
    fn the_bar_is_the_part_of_the_limit_else_of_the_request_and_the_color_is_of_the_limit() {
        let parts: Vec<Option<f64>> = [
            gauge(250, Some(500), Some(1000)),
            gauge(250, Some(500), None),
            gauge(250, None, None),
            gauge(250, Some(0), None),
        ]
        .iter()
        .map(Gauge::part)
        .collect();
        assert_eq!(parts, [Some(0.25), Some(0.5), None, None]);
        assert_eq!(gauge(250, Some(500), Some(1000)).mark(), Some(0.5));
        assert_eq!((gauge(250, Some(500), None).mark(), gauge(250, None, Some(1000)).mark()), (None, None));
        let tones: Vec<Tone> = [790, 800, 949, 950, 2000].map(|usage| gauge(usage, None, Some(1000)).tone()).to_vec();
        assert_eq!(tones, [Tone::Info, Tone::Warn, Tone::Warn, Tone::Error, Tone::Error]);
        assert_eq!(gauge(5000, Some(100), None).tone(), Tone::Info, "no limit: the pod can use its node");
    }

    #[test]
    fn a_chart_shows_millicores_and_mib() {
        assert_eq!((Resource::Cpu.value(250_000_000), Resource::Memory.value(64 * MI)), (250.0, 64.0));
        assert_eq!((Resource::Cpu.text_of(1500.0), Resource::Memory.text_of(64.0)), ("1.5".into(), "64Mi".into()));
        assert_eq!(Resource::Cpu.text_of(-20.0), "0", "the margin below zero");
        assert_eq!((Resource::Cpu.text_of(0.0), Resource::Memory.text_of(0.0)), ("0".into(), "0".into()));
    }

    #[test]
    fn the_words_give_each_value_with_its_part_and_the_age_of_the_sample() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        let clock = Clock::fixed(now);
        let usage = Usage {
            cpu: gauge(250_000_000, Some(500_000_000), None),
            memory: gauge(64 * MI, Some(128 * MI), Some(256 * MI)),
            whole: Whole::Limit,
            at: now - Duration::from_secs(5),
            cover: Cover { sampled: 3, of: 4 },
            current: true,
        };
        assert_eq!(
            usage.hint(Resource::Cpu, &clock),
            "CPU 250m\nrequest 500m (50%)\nno limit\nsample of 07:59:55, 5 s ago\nsum of 3 of 4 pods"
        );
        let node = Usage { whole: Whole::Allocatable, cover: Cover { sampled: 1, of: 1 }, ..usage };
        assert_eq!(
            node.hint(Resource::Memory, &clock),
            "Memory 64Mi\nrequests of its pods 128Mi (50% of allocatable)\nallocatable 256Mi (25%)\nsample of 07:59:55, 5 s ago"
        );
        let cluster = Usage { cover: Cover { sampled: 2, of: 3 }, ..node };
        assert!(
            cluster.hint(Resource::Memory, &clock).ends_with("5 s ago\nsum of 2 of 3 nodes"),
            "a node had no sample"
        );
    }
}
