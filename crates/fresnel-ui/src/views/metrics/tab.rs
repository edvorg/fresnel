//! The Metrics tab of the panel of a Pod, a Node or a workload (plan section 12.4, research
//! 11 section 8.2): the charts of CPU and memory one below the other on one time axis, a
//! choice of the time range, the value of now with its request and limit and the age of its
//! sample, and the state of the list of the cluster when it is not ok.
//!
//! The tab builds its lines again only for a new poll, a new range or a new version of the
//! object, not for each frame: the fixed lines then agree with the value of now, which reads
//! the request and the limit of the object of each frame, also after a resize of a pod.

use std::time::{Duration, SystemTime};

use egui::{Id, RichText, Ui};
use fresnel_core::kinds::{Describe, Object};
use fresnel_core::metrics::Metrics;
use fresnel_core::published::Held;

use super::gauge::Resource;
use super::lines::{self, GAP, Lines};
use super::read::Metered;
use super::{Problems, cells};
use crate::format;
use crate::views::detail::fields::{heading, note};
use crate::views::detail::sections::Context;
use crate::widgets::chart::{self, Chart};
use crate::widgets::table::model::Lead;

/// The time ranges of the charts. The history in memory goes back 24 hours.
const RANGES: [(&str, u64); 5] = [("5 min", 300), ("30 min", 1_800), ("1 h", 3_600), ("4 h", 14_400), ("24 h", 86_400)];

/// The state of the tab: the chosen range and the lines of the last build.
pub(crate) struct MetricsTab {
    /// The time range in seconds.
    range: u64,
    built: Option<Built>,
}

impl Default for MetricsTab {
    fn default() -> Self {
        Self { range: 1_800, built: None }
    }
}

/// The lines of one poll, one range and one version of the object.
struct Built {
    metrics: Held<Metrics>,
    range: u64,
    version: Option<String>,
    lines: [Lines; 2],
}

impl Built {
    /// True when the lines are of this poll, this range and this version of the object.
    fn fits<K: Describe>(&self, metrics: &Held<Metrics>, range: u64, object: &K) -> bool {
        let same_poll = std::ptr::eq::<Metrics>(&*self.metrics, &**metrics);
        same_poll && self.range == range && self.version.as_deref() == object.meta().resource_version.as_deref()
    }
}

impl MetricsTab {
    pub(crate) fn show<K: Describe>(&mut self, ui: &mut Ui, cx: &Context<'_>, object: &K) {
        let palette = cx.palette;
        ui.horizontal(|ui| {
            for (label, seconds) in RANGES {
                ui.selectable_value(&mut self.range, seconds, label);
            }
        });
        ui.add_space(6.0);
        let Some(data) = cx.data else {
            note(ui, palette.warn, "The metrics are not loaded: the cluster is not connected.");
            return;
        };
        let metered = Metered::of(&cx.id.cluster, data);
        let problems = Problems::of(&data.metrics, cx.clock);
        let problem = problems.of_kind(K::ID);
        let usage = metered.usage(&cx.id.key, object);
        let source = format!("From metrics-server, a sample each {}.", format::span(data.metrics.period));
        ui.label(RichText::new(source).color(palette.text_muted));
        if usage.is_none() {
            note(ui, palette.text_muted, "The metrics have no sample of this object yet.");
        }
        let now = cx.clock.now();
        let start = now.checked_sub(Duration::from_secs(self.range)).unwrap_or(SystemTime::UNIX_EPOCH);
        let built = self.built.take().filter(|built| built.fits(&data.metrics, self.range, object));
        let built = built.unwrap_or_else(|| {
            let from = u32::try_from(seconds(start)).unwrap_or(u32::MAX);
            let lines = lines::of(&metered, cx.id, object, from, palette);
            let version = object.meta().resource_version.clone();
            Built { metrics: data.metrics.clone(), range: self.range, version, lines }
        });
        let reach = GAP / 2.0 * data.metrics.period.as_secs_f64();
        let (group, range) =
            (Id::new(("metrics", &cx.id.cluster, &cx.id.key)), seconds(start) as f64..=seconds(now) as f64);
        for (number, (resource, lines)) in Resource::BOTH.into_iter().zip(&built.lines).enumerate() {
            let (more, text) = (&lines.more, |value: f64| resource.text_of(value));
            let (lines, marks) = (&lines.lines, &lines.marks);
            let binary = resource == Resource::Memory;
            let chart = Chart { title: resource.title(), lines, marks, text: &text, reach, binary };
            ui.horizontal(|ui| {
                heading(ui, palette, &resource.title().to_uppercase());
                ui.add_space(8.0);
                chart::legend(ui, palette, (group, number), &chart);
            });
            // The words of the cell of a table, with the reason of an old value first.
            let cell = cells::cell(usage.as_ref(), resource, problem, cx.clock);
            let text = cell.hint.as_deref().unwrap_or(&cell.text).replace('\n', "  ·  ");
            let old = matches!(cell.lead, Lead::Old(_));
            ui.label(RichText::new(text).color(if old { palette.warn } else { palette.text }));
            if let Some(more) = more {
                ui.label(RichText::new(more).color(palette.text_muted));
            }
            chart::show(ui, palette, cx.clock, (group, number), &range, &chart);
            ui.add_space(10.0);
        }
        self.built = Some(built);
    }
}

/// A time as seconds since 1970. A time before 1970 is 0.
fn seconds(time: SystemTime) -> u64 {
    time.duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_secs()
}
