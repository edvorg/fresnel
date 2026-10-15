//! The rows of one frame: from the position of the pane down to its bottom edge, each one
//! with its time and its source in the first row of a line, and a node of the
//! accessibility tree with its text.

use std::collections::{BTreeMap, BTreeSet};

use egui::{Align, Align2, Button, Id, Layout, Painter, Rect, RichText, Sense, Ui, UiBuilder, pos2, vec2};
use fresnel_core::kinds::Kind;
use fresnel_core::logs::{LineKind, LogLine, LogView, SourceView};
use fresnel_core::store::ObjectKey;

use super::paint::{self, Look, Marks};
use super::rows::{Cut, laid_out};
use super::{Input, Metrics, Output, Pane, Placed, SOURCE_COLUMNS};
use crate::format::{PodNames, short_start, thousands};
use crate::theme::{self, Palette, SMALL};

/// The space above the text in a row.
const ABOVE: f32 = 1.5;

/// What the first row of a line draws besides its text.
struct Gutter<'a> {
    painter: &'a Painter,
    palette: &'a Palette,
    input: &'a Input<'a>,
    metrics: &'a Metrics,
}

impl Pane {
    /// Draws the rows on the screen and keeps their places for the pointer.
    pub(super) fn draw(
        &mut self,
        ui: &mut Ui,
        id: Id,
        input: &Input<'_>,
        cut: Cut,
        metrics: &Metrics,
        output: &mut Output,
    ) {
        let palette = theme::palette(ui);
        let shown = input.shown();
        let clip = Rect::from_min_max(pos2(metrics.time, metrics.text.top()), metrics.text.max);
        let painter = ui.painter_at(clip);
        let look = Look { font: metrics.font.clone(), palette, dark: ui.visuals().dark_mode };
        let gutter = Gutter { painter: &painter, palette, input, metrics };
        self.placed.clear();
        self.widest = 0.0;
        let mut y = metrics.text.top() - self.offset;
        let (mut index, mut skip) = (shown.index_from(self.top.line), self.top.row);
        while y < metrics.text.bottom() {
            let Some((number, line)) = shown.line(index) else {
                break;
            };
            let marker = line.kind == LineKind::Marker;
            let hits = match input.search {
                Some(needle) if !marker => needle.places(&line.text, laid_out(&line.text).0),
                _ => Vec::new(),
            };
            let selected = self.selection.and_then(|selection| selection.of_line(number, line.text.len()));
            let current = input.current == Some(number);
            for (row_index, row) in cut.rows(line).iter().enumerate().skip(skip) {
                if y >= metrics.text.bottom() {
                    break;
                }
                let width = metrics.text.right() - metrics.time;
                let full = Rect::from_min_size(pos2(metrics.time, y), vec2(width, metrics.row));
                if marker {
                    painter.rect_filled(full, 0.0, palette.bg_raised);
                }
                let mut label = if row_index == 0 { gutter.draw(line, y) } else { String::new() };
                let x = metrics.text.left() - self.left;
                if row.more {
                    if more(ui, line, pos2(x, y), metrics) {
                        output.full_line = Some(number);
                    }
                } else {
                    let marks = Marks { hits: &hits, current, selected: selected.clone() };
                    let spans = line.spans.as_deref().unwrap_or_default();
                    let galley = painter.layout_job(paint::job(&line.text, &row.bytes, spans, &marks, marker, &look));
                    self.widest = self.widest.max(galley.size().x);
                    painter.galley(pos2(x, y + ABOVE), galley, palette.text);
                    label.push_str(line.text.get(row.bytes.clone()).unwrap_or_default());
                }
                node(ui, full, id.with((number, row_index)), label.trim_end());
                self.placed.push(Placed { line: number, bytes: row.bytes.clone(), at: pos2(x, y) });
                y += metrics.row;
            }
            (index, skip) = (index + 1, 0);
        }
    }
}

impl Gutter<'_> {
    /// Draws the time and the source of a line. Gives their text for the accessibility tree.
    /// A line that came late has its time in the warning color.
    fn draw(&self, line: &LogLine, y: f32) -> String {
        let (palette, metrics) = (self.palette, self.metrics);
        let mut label = String::new();
        if self.input.times {
            let time = self.input.clock.log_time(line.time);
            let color = if line.late { palette.warn } else { palette.text_faint };
            self.painter.text(pos2(metrics.time, y + ABOVE), Align2::LEFT_TOP, &time, metrics.font.clone(), color);
            label.push_str(&time);
            label.push_str(if line.late { " (late) " } else { " " });
        }
        let index = usize::try_from(line.source).ok();
        let source = index.and_then(|index| Some((metrics.names.get(index)?, self.input.view.sources.get(index)?)));
        if let Some((name, source)) = source {
            let slot = line.source as usize % palette.clusters.len();
            let color = palette.clusters.get(slot).copied().unwrap_or(palette.accent);
            self.painter.text(pos2(metrics.source, y + ABOVE), Align2::LEFT_TOP, name, metrics.font.clone(), color);
            // A screen reader reads the full name, which tells the pod and the container.
            label.push_str(&format!("{}/{} ", source.pod.name, source.container));
        }
        label
    }
}

/// The name of each source of a view as the gutter shows it: the short name of its pod
/// (`PodNames`), with the container when the view reads more than one container of that
/// pod. The sources of a view of a pod are its containers. The tab names the object, and
/// the full names of a pod and its container did not fit in the gutter. A longer name than
/// the gutter shows its end, where the names of the pods of one owner differ.
pub(super) fn source_names(view: &LogView) -> Vec<String> {
    // A pod that comes again with its name, as a pod of a StatefulSet, is a new source of
    // the same containers.
    let mut containers: BTreeMap<&ObjectKey, BTreeSet<&str>> = BTreeMap::new();
    for source in view.sources.iter() {
        containers.entry(&source.pod).or_default().insert(&source.container);
    }
    let pods = PodNames::of(containers.keys().map(|pod| &*pod.name));
    let name = |source: &SourceView| {
        let pod = pods.short(&source.pod.name);
        let name = match (view.request.kind, containers.get(&source.pod).map_or(0, BTreeSet::len)) {
            (Kind::Pod, _) => source.container.to_string(),
            (_, 2..) => format!("{pod}/{}", source.container),
            _ => pod.to_owned(),
        };
        short_start(&name, SOURCE_COLUMNS - 1).into_owned()
    };
    view.sources.iter().map(name).collect()
}

/// The columns of the source before the text: the widest name and a space. None for a
/// view whose lines name no source. The gutter is no wider than its names, so the text of
/// a view of short names gets the room.
pub(super) fn source_columns(names: &[String]) -> Option<usize> {
    names.iter().map(|name| name.chars().count() + 1).max()
}

/// A node of the accessibility tree for a row: a screen reader reads the text that the pane
/// paints itself, and a test finds the rows by their role. egui gives a place only to the
/// node of a widget that takes the focus, so the row sets its own.
fn node(ui: &Ui, rect: Rect, id: Id, text: &str) {
    let response = ui.interact(rect, id, Sense::hover());
    ui.ctx().accesskit_node_builder(response.id, |node| {
        node.set_role(egui::accesskit::Role::ListItem);
        node.set_label(text);
        let (min, max) = (rect.min, rect.max);
        node.set_bounds(egui::accesskit::Rect {
            x0: min.x.into(),
            y0: min.y.into(),
            x1: max.x.into(),
            y1: max.y.into(),
        });
    });
}

/// The row after the laid-out part of a long line: a button that shows the full line, with
/// the note of a line that the core cut. Gives true for a click.
fn more(ui: &mut Ui, line: &LogLine, at: egui::Pos2, metrics: &Metrics) -> bool {
    let palette = theme::palette(ui);
    let size = thousands(line.text.len() as u64);
    let text = if line.cut {
        format!("Show the full line: the first {size} bytes, the most that the app keeps of one line")
    } else {
        format!("Show the full line: {size} bytes")
    };
    let rect = Rect::from_min_size(at, vec2(metrics.text.width(), metrics.row)).intersect(metrics.text);
    let button = Button::new(RichText::new(text).size(SMALL).color(palette.accent)).frame(false);
    let place = UiBuilder::new().max_rect(rect).layout(Layout::left_to_right(Align::Center));
    ui.scope_builder(place, |ui| ui.add(button)).inner.clicked()
}

#[cfg(test)]
mod tests {
    use fresnel_core::cluster::ClusterId;
    use fresnel_core::logs::{Lines, LogOptions, LogRequest, PodsState, SourceState};

    use super::*;

    /// A view of the object `owner` of this kind with a source for each pod and container.
    fn view(kind: Kind, owner: &str, sources: &[(&str, &str)]) -> LogView {
        let source = |(pod, container): &(&str, &str)| SourceView {
            pod: ObjectKey::new("shop", pod),
            container: (*container).into(),
            state: SourceState::Streaming,
            lines: 0,
            repeated: 0,
        };
        let key = ObjectKey::new("shop", owner);
        let request =
            LogRequest { cluster: ClusterId::new("ams"), kind, key, uid: "uid".into(), options: LogOptions::default() };
        LogView {
            request,
            pods: PodsState::Following,
            sources: sources.iter().map(source).collect(),
            lines: Lines::new(0, Vec::new()),
            dropped: 0,
            bytes: 0,
            unstreamed: 0,
            restarts: 0,
        }
    }

    fn names(kind: Kind, owner: &str, sources: &[(&str, &str)]) -> Vec<String> {
        source_names(&view(kind, owner, sources))
    }

    #[test]
    fn a_source_has_the_short_name_of_its_pod_and_its_container_when_its_pod_has_more() {
        let (gn8jp, k8q55) = ("checkout-api-7d9c5b8f6-gn8jp", "checkout-api-7d9c5b8f6-k8q55");
        let deployment = |sources: &[(&str, &str)]| names(Kind::Deployment, "checkout-api", sources);
        assert_eq!(deployment(&[(gn8jp, "api"), (k8q55, "api")]), ["gn8jp", "k8q55"]);
        assert_eq!(
            deployment(&[(gn8jp, "web"), (gn8jp, "side"), (k8q55, "web"), (k8q55, "side")]),
            ["gn8jp/web", "gn8jp/side", "k8q55/web", "k8q55/side"]
        );
        assert_eq!(
            deployment(&[(gn8jp, "api"), ("checkout-api-5f6b7c8d9-ab12c", "api")]),
            ["7d9c5b8f6-gn8jp", "5f6b7c8d9-ab12c"],
            "a rollout keeps the hash of each ReplicaSet"
        );
        assert_eq!(names(Kind::Pod, gn8jp, &[(gn8jp, "app"), (gn8jp, "istio-proxy")]), ["app", "istio-proxy"]);
    }

    #[test]
    fn a_pod_of_a_stateful_set_keeps_its_name_and_a_long_name_shows_its_end() {
        let sources = [("postgres-0", "postgres"), ("postgres-1", "postgres"), ("postgres-0", "postgres")];
        assert_eq!(
            names(Kind::StatefulSet, "postgres", &sources),
            ["postgres-0", "postgres-1", "postgres-0"],
            "a pod that came again with its name has the same containers"
        );
        let master = |number: u32| format!("elasticsearch-master-{number}");
        let pods: Vec<String> = (0..3).map(master).collect();
        let sources: Vec<(&str, &str)> = pods.iter().map(|pod| (pod.as_str(), "elasticsearch")).collect();
        let long = names(Kind::StatefulSet, "elasticsearch-master", &sources);
        assert_eq!(long, pods, "three pods, three names");
        let two: Vec<(&str, &str)> =
            pods.iter().flat_map(|pod| [(pod.as_str(), "elasticsearch"), (pod.as_str(), "exporter")]).collect();
        let cut = names(Kind::StatefulSet, "elasticsearch-master", &two);
        assert_eq!(cut.first().map(String::as_str), Some("…icsearch-master-0/elasticsearch"));
        assert_eq!(cut.iter().collect::<BTreeSet<_>>().len(), 6, "each source has its own name: {cut:?}");
        assert!(cut.iter().all(|name| name.chars().count() <= SOURCE_COLUMNS), "{cut:?}");
    }

    #[test]
    fn the_gutter_is_as_wide_as_its_widest_name_and_a_space() {
        let names = |names: &[&str]| names.iter().map(|name| (*name).to_owned()).collect::<Vec<String>>();
        assert_eq!(source_columns(&names(&["gn8jp", "gn8jp/istio-proxy"])), Some(18));
        assert_eq!(source_columns(&[]), None, "a view with one source");
    }
}
