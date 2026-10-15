//! The rows of one frame: from the position of the pane down to its bottom edge, each one
//! with its time and its source in the first row of a line, and a node of the
//! accessibility tree with its text.

use std::collections::BTreeMap;

use egui::{Align, Align2, Button, Id, Layout, Painter, Rect, RichText, Sense, Ui, UiBuilder, pos2, vec2};
use fresnel_core::kinds::Kind;
use fresnel_core::logs::{LineKind, LogLine, LogView, SourceView};
use fresnel_core::store::ObjectKey;

use super::paint::{self, Look, Marks};
use super::rows::{Cut, laid_out};
use super::{Input, Metrics, Output, Pane, Placed, SOURCE_COLUMNS};
use crate::format::{pod_end, short, thousands};
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
            let text = short(name, SOURCE_COLUMNS - 1);
            self.painter.text(pos2(metrics.source, y + ABOVE), Align2::LEFT_TOP, &text, metrics.font.clone(), color);
            // A screen reader reads the full name, which tells the pod and the container.
            label.push_str(&format!("{}/{} ", source.pod.name, source.container));
        }
        label
    }
}

/// The name of each source of a view in the gutter: the end of the name of its pod
/// (`pod_end`), with the container when the view reads more than one container of that pod.
/// The tab names the object, and the full names of a pod and its container did not fit in
/// the gutter. The sources of a view of a pod are its containers.
pub(super) fn source_names(view: &LogView) -> Vec<String> {
    let mut containers: BTreeMap<&ObjectKey, usize> = BTreeMap::new();
    for source in view.sources.iter() {
        *containers.entry(&source.pod).or_default() += 1;
    }
    let name = |source: &SourceView| {
        let pod = pod_end(&source.pod.name, &view.request.key.name);
        match (view.request.kind, containers.get(&source.pod).copied().unwrap_or_default()) {
            (Kind::Pod, _) => source.container.to_string(),
            (_, 2..) => format!("{pod}/{}", source.container),
            _ => pod.to_owned(),
        }
    };
    view.sources.iter().map(name).collect()
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
