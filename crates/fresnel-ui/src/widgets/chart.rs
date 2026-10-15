//! The charts of the metrics (plan section 12.4). Only this module names `egui_plot` (plan
//! section 7.2), so an upgrade of that crate changes this file.
//!
//! It adds what the crate lacks (research 11 section 7.4): a time axis, a gap where samples
//! are missing, a tooltip with the value of each line at the time of the pointer, and fixed
//! lines for a request and a limit. The charts of one stack share their time axis and their
//! cursor. The points of a line are built one time for each new sample, not for each frame.
//!
//! - `time`: the marks of the time axis. `value`: the marks of a decimal value axis.
//! - `legend`: the names of the lines beside the heading of a chart; a click hides one. It
//!   gives the `Shown` that `show` draws.

mod legend;
mod time;
mod value;

use std::ops::{Range, RangeInclusive};
use std::time::{Duration, SystemTime};

use egui::{Color32, Id, RichText, Ui};
use egui_plot::{GridMark, HLine, LineStyle, Plot, PlotPoint, PlotPoints, log_grid_spacer, uniform_grid_spacer};

pub(crate) use self::legend::Shown;
use crate::format::Clock;
use crate::theme::Palette;
use crate::widgets::{self, dot};

/// The width of the value axis of each chart, so the charts of one time axis line up.
const VALUE_AXIS: f32 = 52.0;

/// The most lines that a chart should have: the tessellation of 30 long lines costs 4 ms
/// for each frame (research 11 section 7.4).
pub(crate) const MOST_LINES: usize = 11;

/// One line of a chart: the samples of one series in the sequence of their times.
pub(crate) struct Line {
    /// The name in the tooltip.
    name: String,
    /// The name in the legend: `name`, or a short form of it.
    legend: String,
    color: Color32,
    /// x is the time in seconds since 1970, y the value in the unit of the chart.
    points: Vec<PlotPoint>,
    /// The parts with no gap, as ranges of `points`.
    runs: Vec<Range<usize>>,
}

impl Line {
    /// A line through these points, in the sequence of their times. `None` is a gap: the
    /// line stops there and starts again at the next point.
    pub(crate) fn new(name: String, color: Color32, points: impl IntoIterator<Item = Option<[f64; 2]>>) -> Self {
        let (mut all, mut runs, mut start) = (Vec::new(), Vec::new(), 0);
        for point in points {
            match point {
                Some([x, y]) => all.push(PlotPoint::new(x, y)),
                None => {
                    runs.extend((all.len() > start).then_some(start..all.len()));
                    start = all.len();
                }
            }
        }
        runs.extend((all.len() > start).then_some(start..all.len()));
        Self { legend: name.clone(), name, color, points: all, runs }
    }

    /// The line with a shorter name in the legend, so the names of many lines fit beside
    /// the heading of the chart; the tooltip keeps the full name.
    pub(crate) fn with_legend(self, legend: &str) -> Self {
        Self { legend: legend.to_owned(), ..self }
    }

    /// The value at the time `x`: the nearest sample. Between two samples of one part it is
    /// always a value; at the end of a part only `reach` seconds away from its sample, and
    /// in a gap none.
    fn at(&self, x: f64, reach: f64) -> Option<f64> {
        let after = self.points.partition_point(|point| point.x < x);
        let inside = self.runs.iter().any(|run| run.start < after && after < run.end);
        let near = [after.checked_sub(1), Some(after)].into_iter().flatten().filter_map(|index| self.points.get(index));
        let nearest = near.min_by(|a, b| (a.x - x).abs().total_cmp(&(b.x - x).abs()));
        nearest.filter(|point| inside || (point.x - x).abs() <= reach).map(|point| point.y)
    }
}

/// A fixed line across a chart: a request, a limit or the allocatable part of a node.
pub(crate) struct Mark {
    pub(crate) name: String,
    pub(crate) value: f64,
    pub(crate) color: Color32,
}

/// One chart of a stack.
pub(crate) struct Chart<'a> {
    /// The name of the chart for a screen reader, and the first line of its tooltip.
    pub(crate) title: &'a str,
    pub(crate) lines: &'a [Line],
    pub(crate) marks: &'a [Mark],
    /// The text of a value in the unit of the chart, for the axis and the tooltip.
    pub(crate) text: &'a dyn Fn(f64) -> String,
    /// How far from the end of a part of a line a sample still tells the value, in seconds.
    pub(crate) reach: f64,
    /// True for a unit of powers of 2, such as MiB: the marks of the value axis are then
    /// on 1, 2, 4 or 8 Gi, not on 9.77 Gi.
    pub(crate) binary: bool,
}

/// Draws the legend of one chart of a stack where the caller puts it, as beside the heading
/// of the chart: there it takes no line of its own. Gives what `show` draws in this frame.
pub(crate) fn legend(ui: &mut Ui, palette: &Palette, (group, number): (Id, usize), chart: &Chart<'_>) -> Shown {
    legend::show(ui, palette, group.with(number), (chart.lines, chart.marks))
}

/// Draws one chart over the times of `range`, in seconds since 1970. The charts of one
/// `group` share their time axis and their cursor; the range moves with the clock, so a
/// chart has no zoom and no drag. `number` tells the charts of a group apart. A line that
/// `shown` hides leaves the plot and its bounds, and the line of the entry under the pointer
/// is thicker and in front.
pub(crate) fn show(
    ui: &mut Ui,
    palette: &Palette,
    clock: &Clock,
    (group, number): (Id, usize),
    range: &RangeInclusive<f64>,
    (chart, shown): (&Chart<'_>, &Shown),
) {
    let id = group.with(number);
    let label = |mark: GridMark, _: &RangeInclusive<f64>| {
        let time = time_of(mark.value);
        if mark.step_size < 60.0 { clock.time_of_day(time) } else { clock.hour_minute(time) }
    };
    let values =
        if chart.binary { log_grid_spacer(2) } else { uniform_grid_spacer(|input| value::steps(input.base_step_size)) };
    let plot = Plot::new(id)
        .height(150.0)
        .link_axis(group, [true, false])
        .link_cursor(group, [true, false])
        .allow_zoom(false)
        .allow_drag(false)
        .allow_scroll(false)
        .allow_boxed_zoom(false)
        .allow_double_click_reset(false)
        .include_y(0.0)
        .y_axis_min_width(VALUE_AXIS)
        .y_grid_spacer(values)
        .x_grid_spacer(uniform_grid_spacer(|input| time::steps(input.base_step_size)))
        .x_axis_formatter(label)
        .y_axis_formatter(|mark, _| (chart.text)(mark.value));
    let response = plot.show(ui, |plot| {
        plot.set_plot_bounds_x(range.clone());
        for line in shown.lines(chart.lines) {
            // `egui_plot` draws a highlighted item thicker, and after the others.
            let thick = shown.hovers_line(line);
            for run in &line.runs {
                let points = PlotPoints::Borrowed(line.points.get(run.clone()).unwrap_or_default());
                let part = egui_plot::Line::new(line.legend.as_str(), points).color(line.color).width(1.5);
                plot.line(part.highlight(thick));
            }
        }
        for mark in shown.marks(chart.marks) {
            let fixed = HLine::new(mark.name.as_str(), mark.value).color(mark.color).style(LineStyle::dashed_dense());
            plot.hline(fixed.highlight(shown.hovers_mark(mark)));
        }
    });
    widgets::name(&response.response, &format!("{} chart", chart.title));
    if let Some(pointer) = response.response.hover_pos() {
        let x = response.transform.value_from_position(pointer).x;
        response.response.on_hover_ui_at_pointer(|ui| tooltip(ui, palette, clock, (chart, shown), x));
    }
}

/// The value of each shown line and each shown mark at the time of the pointer: `egui_plot`
/// gives the value of the nearest line only. A line that the user hid has no value there.
fn tooltip(ui: &mut Ui, palette: &Palette, clock: &Clock, (chart, shown): (&Chart<'_>, &Shown), x: f64) {
    ui.label(RichText::new(format!("{} at {}", chart.title, clock.time_of_day(time_of(x)))).strong());
    for line in shown.lines(chart.lines) {
        let value = line.at(x, chart.reach).map_or_else(|| "no sample".to_owned(), |value| (chart.text)(value));
        ui.horizontal(|ui| {
            dot(ui, line.color);
            ui.label(format!("{}: {value}", line.name));
        });
    }
    for mark in shown.marks(chart.marks) {
        ui.horizontal(|ui| {
            dot(ui, mark.color);
            ui.label(RichText::new(format!("{}: {}", mark.name, (chart.text)(mark.value))).color(palette.text_muted));
        });
    }
}

/// The time of a value of the time axis. A value before 1970 or not a number is 1970.
fn time_of(seconds: f64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::try_from_secs_f64(seconds).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(points: &[Option<[f64; 2]>]) -> Line {
        Line::new("app".to_owned(), Color32::WHITE, points.iter().copied())
    }

    #[test]
    fn a_gap_splits_a_line_into_parts_and_has_no_value() {
        let gapped = line(&[None, Some([0.0, 1.0]), Some([15.0, 2.0]), None, None, Some([60.0, 3.0]), None]);
        assert_eq!(gapped.runs, [0..2, 2..3]);
        assert_eq!(line(&[]).runs, Vec::<Range<usize>>::new());
        let values: Vec<Option<f64>> =
            [-20.0, -5.0, 7.0, 9.0, 37.0, 50.0, 75.0, 76.0].map(|x| gapped.at(x, 15.0)).to_vec();
        assert_eq!(values, [None, Some(1.0), Some(1.0), Some(2.0), None, Some(3.0), Some(3.0), None]);
        let minutes = line(&[Some([0.0, 1.0]), Some([60.0, 2.0]), Some([120.0, 3.0])]);
        assert_eq!(
            [31.0, 89.0, 136.0].map(|x| minutes.at(x, 15.0)),
            [Some(2.0), Some(2.0), None],
            "the samples of minutes"
        );
    }

    /// A line from 0 to 60 seconds on the value `y`.
    fn flat(name: &str, color: Color32, y: f64) -> Line {
        Line::new(name.to_owned(), color, [Some([0.0, y]), Some([60.0, y])])
    }

    /// Draws the legend and the chart of these lines and fixed lines, lets `act` use the
    /// legend, and gives the top of the value axis and the shapes of the last frame.
    fn drawn(
        (lines, marks): (&[Line], &[Mark]),
        act: impl FnOnce(&mut egui_kittest::Harness<'_>),
    ) -> (f64, Vec<egui::Shape>) {
        let top = std::cell::Cell::new(f64::NAN);
        let mut harness = egui_kittest::Harness::new_ui(|ui| {
            let palette = crate::theme::palette(ui);
            let text = |value: f64| value.to_string();
            let chart = Chart { title: "CPU", lines, marks, text: &text, reach: 15.0, binary: false };
            let group = Id::new("charts");
            let shown = legend(ui, palette, (group, 0), &chart);
            let clock = Clock::fixed(SystemTime::UNIX_EPOCH);
            show(ui, palette, &clock, (group, 0), &(0.0..=60.0), (&chart, &shown));
            // The place of the memory of the plot, as `Plot::show` makes it.
            let memory = egui_plot::PlotMemory::load(ui.ctx(), ui.make_persistent_id(Id::new(group.with(0))));
            top.set(memory.map_or(f64::NAN, |memory| memory.bounds().max()[1]));
        });
        harness.run();
        act(&mut harness);
        let shapes = harness.output().shapes.iter().map(|clipped| clipped.shape.clone()).collect();
        drop(harness);
        (top.get(), shapes)
    }

    /// Clicks these entries of the legend, with a frame after each click.
    fn click(harness: &mut egui_kittest::Harness<'_>, clicks: &[(&str, egui::Modifiers)]) {
        use egui_kittest::kittest::Queryable as _;

        for (name, modifiers) in clicks {
            harness.get_by_label(name).click_modifiers(*modifiers);
            harness.run();
        }
    }

    /// The top of the value axis of a chart with the lines "app" near zero and "burst" near
    /// 8, and the fixed line "limit" on 4, after these clicks on its legend.
    fn top_after(clicks: &[(&str, egui::Modifiers)]) -> f64 {
        let lines = [flat("app", Color32::WHITE, 0.02), flat("burst", Color32::WHITE, 8.0)];
        let marks = [Mark { name: "limit".to_owned(), value: 4.0, color: Color32::RED }];
        drawn((&lines, &marks), |harness| click(harness, clicks)).0
    }

    /// The widths of the strokes of the lines of this color: a line of a chart is a path.
    fn widths(shapes: &[egui::Shape], color: Color32) -> Vec<f32> {
        let mut found = Vec::new();
        for shape in shapes {
            match shape {
                egui::Shape::Vec(inner) => found.extend(widths(inner, color)),
                egui::Shape::Path(path) if path.stroke.color == egui::epaint::ColorMode::Solid(color) => {
                    found.push(path.stroke.width);
                }
                _ => {}
            }
        }
        found
    }

    #[test]
    fn a_line_that_the_legend_hides_leaves_the_bounds_of_the_value_axis() {
        let none = egui::Modifiers::NONE;
        assert!(top_after(&[]) > 8.0, "each line: {}", top_after(&[]));
        let burst = top_after(&[("burst", none)]);
        assert!((4.0..8.0).contains(&burst), "the limit is the top: {burst}");
        let both = top_after(&[("burst", none), ("limit", none)]);
        assert!(both < 1.0, "the line near zero fills the chart: {both}");
        let alone = top_after(&[("app", egui::Modifiers::ALT)]);
        assert!(alone < 1.0, "an Alt+click shows the line alone: {alone}");
        let again = top_after(&[("app", egui::Modifiers::ALT), ("app", egui::Modifiers::ALT)]);
        assert!(again > 8.0, "a second Alt+click shows each line again: {again}");
    }

    #[test]
    fn the_pointer_on_a_legend_entry_draws_its_line_thicker() {
        use egui_kittest::kittest::Queryable as _;

        let (app, burst) = (Color32::from_rgb(10, 120, 200), Color32::from_rgb(220, 90, 40));
        let lines = [flat("app", app, 1.0), flat("burst", burst, 2.0)];
        let (_, before) = drawn((&lines, &[]), |_| {});
        assert_eq!((widths(&before, app), widths(&before, burst)), (vec![1.5], vec![1.5]));
        let (_, hovered) = drawn((&lines, &[]), |harness| {
            harness.get_by_label("burst").hover();
            harness.run();
        });
        assert_eq!((widths(&hovered, app), widths(&hovered, burst)), (vec![1.5], vec![3.0]), "only burst is thicker");
    }

    /// A chart with no line, or with each entry hidden, draws no line and keeps a value axis.
    /// An Alt+click on the only entry leaves it shown: no other entry is there to hide.
    #[test]
    fn a_chart_with_no_shown_line_has_a_value_axis_and_an_alt_click_on_its_only_line_shows_it() {
        let (top, shapes) = drawn((&[], &[]), |_| {});
        assert!(top.is_finite() && widths(&shapes, Color32::WHITE).is_empty(), "no line: {top}");
        let line = [flat("app", Color32::WHITE, 3.0)];
        let marks = [Mark { name: "limit".to_owned(), value: 4.0, color: Color32::RED }];
        let none = egui::Modifiers::NONE;
        let (top, shapes) = drawn((&line, &marks), |harness| click(harness, &[("app", none), ("limit", none)]));
        assert!(top.is_finite() && widths(&shapes, Color32::WHITE).is_empty(), "each entry hidden: {top}");
        let (top, shapes) = drawn((&line, &[]), |harness| click(harness, &[("app", egui::Modifiers::ALT)]));
        // The pointer stays on the entry, so the line is thicker.
        assert_eq!(widths(&shapes, Color32::WHITE), [3.0], "the only line stays");
        assert!(top > 3.0, "{top}");
    }

    #[test]
    fn a_time_of_the_axis_that_is_not_a_time_is_1970() {
        assert_eq!((time_of(-5.0), time_of(f64::NAN)), (SystemTime::UNIX_EPOCH, SystemTime::UNIX_EPOCH));
        assert_eq!(time_of(90.5), SystemTime::UNIX_EPOCH + Duration::from_millis(90_500));
    }
}
