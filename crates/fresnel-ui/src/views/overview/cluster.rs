//! The blocks of the Cluster overview: one for each selected cluster, in the sequence of the
//! clusters (research 11 section 8.3). A block has the state of the cluster, the version of
//! its server, its nodes, its pods by phase, its CPU and memory against the allocatable part
//! of its nodes, and its warning Events with the newest ones.
//!
//! A count of a store that is not live has the mark of that store, and the CPU and memory
//! have the state of the sampler: each cluster shows its own state, and the other blocks
//! keep their values (weaknesses 3 and 4 of plan section 6). A count is a link to the table
//! of its kind, with the "problems" filter for a count of problems.

use egui::{Frame, Grid, Label, RichText, Ui};
use fresnel_core::app::{ClusterData, ClusterRow};
use fresnel_core::cluster::{LinkState, ServerVersion, VersionState};
use fresnel_core::index::last_seen;
use fresnel_core::kinds::Kind;
use fresnel_core::store::{ObjectKey, StoreStatus};
use k8s_openapi::api::core::v1::Event;

use super::Jump;
use super::count::{Counted, Nodes};
use crate::format::{self, Clock};
use crate::shell::Scene;
use crate::theme::{self, PAD, Palette};
use crate::views::detail::fields::{NONE, field, heading, note};
use crate::views::metrics::Problems;
use crate::views::metrics::gauge::{Resource, Usage};
use crate::views::metrics::read;
use crate::views::objects::{Link, count_mark};
use crate::widgets::table::cells::ring;
use crate::widgets::tone::Tone;
use crate::widgets::{self, dot};

/// The most characters of the message of an Event in its line.
const MESSAGE_LIMIT: usize = 160;
/// The reason of an old value of a list that works.
const OLD: &str = "old: the last answer of metrics-server had no sample of the nodes";
/// The size of a bar of the CPU or the memory.
const BAR: egui::Vec2 = egui::vec2(160.0, 8.0);

pub(super) fn show(ui: &mut Ui, scene: &Scene<'_>, counted: &[Counted]) -> Option<Jump> {
    let palette = theme::palette(ui);
    let selected: Vec<&ClusterRow> = scene.view.clusters.iter().filter(|row| row.selected).collect();
    if selected.is_empty() {
        ui.label(RichText::new("No cluster is selected. Select a cluster in the top bar.").color(palette.text_muted));
    }
    let mut jump = None;
    for row in selected {
        let counts = counted.iter().find(|counts| counts.cluster == row.id);
        let block = ui.push_id(row.id.as_str(), |ui| block(ui, palette, scene, row, counts));
        jump = block.inner.or(jump);
        ui.add_space(PAD);
    }
    jump
}

/// The block of one cluster.
fn block(
    ui: &mut Ui,
    palette: &Palette,
    scene: &Scene<'_>,
    row: &ClusterRow,
    counts: Option<&Counted>,
) -> Option<Jump> {
    let frame = Frame::new().fill(palette.bg_surface).stroke((1.0, palette.border)).corner_radius(6).inner_margin(10);
    let shown = frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            dot(ui, scene.colors.color(&row.id, palette));
            ui.label(RichText::new(row.id.as_str()).strong());
            let (tone, state) = Tone::of_health(row.health());
            ui.label(RichText::new(state).color(tone.color(palette)));
            if let Some(reason) = row.reason() {
                ui.add(Label::new(RichText::new(reason).color(palette.text_muted)).truncate());
            }
        });
        ui.add_space(4.0);
        let Some(data) = &row.data else {
            let text = match &row.link {
                LinkState::Failed(error) => format!("The connect did not start: {error}"),
                LinkState::Off | LinkState::On(_) => "The connect starts.".to_owned(),
            };
            ui.label(RichText::new(text).color(palette.text_muted));
            return None;
        };
        field(ui, palette, "Version", |ui| version(ui, palette, &data.version, scene.clock));
        let Some(counts) = counts else {
            ui.label(RichText::new("The worker thread counts the objects.").color(palette.text_faint));
            return None;
        };
        let mut jump = None;
        field(ui, palette, "Nodes", |ui| jump = nodes(ui, palette, scene.clock, data, counts.nodes.as_ref()));
        field(ui, palette, "Pods", |ui| jump = pods(ui, palette, scene.clock, data, counts).or(jump.take()));
        usage(ui, palette, scene.clock, row, counts.nodes.as_ref());
        warnings(ui, palette, scene.clock, row, counts).or(jump)
    });
    shown.inner
}

/// The version of the server, the version of before during a failed read, or the state.
fn version(ui: &mut Ui, palette: &Palette, version: &ServerVersion, clock: &Clock) {
    let known = version.version.as_deref();
    let (tone, text) = match &version.state {
        VersionState::Reading => (Tone::Faint, "the read is on its way".to_owned()),
        VersionState::Read(_) => (Tone::Normal, known.unwrap_or(NONE).to_owned()),
        VersionState::Failed { error, at } => {
            let failed = format!("the read of {} failed: {error}", clock.time_of_day(*at));
            (
                Tone::Warn,
                known.map_or_else(|| format!("not known: {failed}"), |known| format!("{known}, old: {failed}")),
            )
        }
        VersionState::InternalError(reason) => {
            (Tone::Error, format!("{}: internal error: {reason}", known.unwrap_or(NONE)))
        }
    };
    ui.add(Label::new(RichText::new(text).color(tone.color(palette))).wrap());
}

/// A count as a link to the table of its kind. Gives the jump of a click.
fn count_link(ui: &mut Ui, text: &str, kind: Kind, problems: bool) -> Option<Jump> {
    ui.link(text).clicked().then_some(Jump::Table { kind, problems })
}

/// The mark of a count of a store that is not live, after the count.
fn store_mark(ui: &mut Ui, palette: &Palette, status: Option<&StoreStatus>, clock: &Clock) {
    let mark = match status {
        Some(status) => count_mark(status, clock),
        None => Some("not loaded: the store starts".to_owned()),
    };
    if let Some(mark) = mark {
        ring(ui, palette);
        ui.add(Label::new(RichText::new(mark).color(palette.warn)).truncate());
    }
}

/// The parts of a line of counts, with a space between two of them.
fn parts<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        add(ui)
    })
    .inner
}

fn nodes(ui: &mut Ui, palette: &Palette, clock: &Clock, data: &ClusterData, nodes: Option<&Nodes>) -> Option<Jump> {
    let mut jump = None;
    parts(ui, |ui| {
        if let Some(nodes) = nodes {
            let ready = format!("{} of {} ready", nodes.ready, format::count(nodes.health.total().into(), "node"));
            jump = count_link(ui, &ready, Kind::Node, false);
            if nodes.cordoned > 0 {
                ui.label(format!("· {} cordoned", nodes.cordoned));
            }
            jump = problems(ui, palette, nodes.health.problems(), Kind::Node).or(jump.take());
        }
        store_mark(ui, palette, data.objects.status(Kind::Node), clock);
    });
    jump
}

/// The link to the objects of a kind that the "problems" filter shows, when it has some.
fn problems(ui: &mut Ui, palette: &Palette, count: u32, kind: Kind) -> Option<Jump> {
    (count > 0).then(|| {
        ui.label(RichText::new("·").color(palette.text_faint));
        count_link(ui, &format!("{count} with problems"), kind, true)
    })?
}

fn pods(ui: &mut Ui, palette: &Palette, clock: &Clock, data: &ClusterData, counts: &Counted) -> Option<Jump> {
    let mut jump = None;
    parts(ui, |ui| {
        jump = count_link(ui, &format::count(counts.pods.total().into(), "pod"), Kind::Pod, false);
        for (phase, count) in counts.phases.iter().filter(|(_, count)| *count > 0) {
            ui.label(format!("· {count} {}", phase.to_lowercase()));
        }
        jump = problems(ui, palette, counts.pods.problems(), Kind::Pod).or(jump.take());
        store_mark(ui, palette, Some(&data.pods.status(Kind::Pod)), clock);
    });
    jump
}

/// The CPU and the memory of the cluster: the sum of its nodes against their allocatable
/// part, with the requests on them as the mark of the bar and the age of the sample. An old
/// value has the mark of the rows, and the reason follows once: the state of the list of
/// the nodes of the sampler when it is not ok.
fn usage(ui: &mut Ui, palette: &Palette, clock: &Clock, row: &ClusterRow, nodes: Option<&Nodes>) {
    let Some(data) = &row.data else {
        return;
    };
    let (requests, most) = nodes.map(|nodes| (nodes.requests, nodes.allocatable)).unwrap_or_default();
    let usage = read::of_cluster(&data.metrics, &row.id, requests, most);
    let problems = Problems::of(&data.metrics, clock);
    let problem = problems.of_kind(Kind::Node);
    let Some(usage) = usage else {
        let text = problem.unwrap_or("The metrics have no sample of this cluster yet.");
        let tone = if problem.is_some() { palette.warn } else { palette.text_faint };
        field(ui, palette, "CPU, memory", |ui| note(ui, tone, text));
        return;
    };
    let reason = problem.or((!usage.current).then_some(OLD));
    for resource in Resource::BOTH {
        field(ui, palette, resource.title(), |ui| meter(ui, palette, clock, &usage, resource, reason.is_some()));
    }
    if let Some(reason) = reason {
        field(ui, palette, "", |ui| note(ui, palette.warn, reason));
    }
}

/// One resource: the bar and the words of the values, with the mark of an old value.
fn meter(ui: &mut Ui, palette: &Palette, clock: &Clock, usage: &Usage, resource: Resource, old: bool) {
    let gauge = usage.of(resource);
    let hint = usage.hint(resource, clock);
    parts(ui, |ui| {
        if old {
            ring(ui, palette);
        }
        let mark = gauge.mark().map(|mark| mark as f32);
        let tone = if old { Tone::Faint } else { gauge.tone() };
        widgets::bar(ui, palette, BAR, (gauge.part().unwrap_or_default() as f32, mark), tone.color(palette));
        let mut text = resource.text(gauge.usage);
        if let Some(most) = gauge.most.filter(|most| *most > 0) {
            let part = gauge.usage as f64 / most as f64 * 100.0;
            text.push_str(&format!(" of {} allocatable ({part:.0}%)", resource.text(most)));
        }
        if let Some(request) = gauge.request {
            text.push_str(&format!(" · requests {}", resource.text(request)));
        }
        text.push_str(&format!(" · sample {} ago", format::span(clock.since(usage.at))));
        ui.add(Label::new(text).truncate()).on_hover_text(hint);
    });
}

/// The count of the warning Events with its link, and the newest ones with their objects as links.
fn warnings(ui: &mut Ui, palette: &Palette, clock: &Clock, row: &ClusterRow, counts: &Counted) -> Option<Jump> {
    ui.add_space(4.0);
    heading(ui, palette, "WARNING EVENTS");
    let status = row.data.as_ref().and_then(|data| data.objects.status(Kind::Event));
    let mut jump = None;
    parts(ui, |ui| {
        if let Some(warnings) = &counts.warnings {
            jump = count_link(ui, &format::count(warnings.count.into(), "warning"), Kind::Event, true);
        }
        store_mark(ui, palette, status, clock);
    });
    let newest = counts.warnings.as_ref().map(|warnings| warnings.newest.as_slice()).unwrap_or_default();
    if newest.is_empty() {
        return jump;
    }
    Grid::new("warnings").num_columns(4).spacing([12.0, 4.0]).show(ui, |ui| {
        for event in newest {
            let seen = last_seen(event).map(|time| format::span(clock.since(time.into())));
            ui.label(
                RichText::new(seen.map_or_else(|| NONE.to_owned(), |seen| format!("{seen} ago")))
                    .color(palette.text_muted),
            );
            jump = object(ui, row, event).or(jump.take());
            ui.label(RichText::new(event.reason.as_deref().unwrap_or(NONE)).color(palette.warn));
            let message = event.message.as_deref().map(str::trim).unwrap_or_default();
            ui.add(Label::new(format::short(message, MESSAGE_LIMIT)).truncate());
            ui.end_row();
        }
    });
    jump
}

/// The object of an Event as a link to its panel, or as a text for a kind that is not in
/// the scope of the app.
fn object(ui: &mut Ui, row: &ClusterRow, event: &Event) -> Option<Jump> {
    let reference = &event.involved_object;
    let (kind, name) = (reference.kind.as_deref().unwrap_or(NONE), reference.name.as_deref().unwrap_or(NONE));
    let text = format!("{kind} {name}");
    let Some(kind) = Kind::named(kind).filter(|_| reference.name.is_some()) else {
        ui.label(text);
        return None;
    };
    let namespace = reference.namespace.as_deref().or(event.metadata.namespace.as_deref()).unwrap_or_default();
    let namespace = if kind.info().namespaced { namespace } else { "" };
    let link = Link { cluster: row.id.clone(), kind, key: ObjectKey::new(namespace, name) };
    ui.link(text).clicked().then_some(Jump::Object(link))
}
