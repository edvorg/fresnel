//! The charts of the metrics (plan section 12.4). Only this module names `egui_plot` (plan
//! section 7.2), so an upgrade of that crate changes this file.
//!
//! It adds what the crate lacks (research 11 section 7.4): a time axis, a gap where samples
//! are missing, a tooltip with the value of each line at the time of the pointer, and fixed
//! lines for a request and a limit. The charts of one stack share their time axis and their
//! cursor. The points of a line are built one time for each new sample, not for each frame.
//!
//! - `time`: the marks of the time axis.

mod time;

use std::ops::{Range, RangeInclusive};
use std::time::{Duration, SystemTime};

use egui::{Color32, Id, RichText, Ui};
use egui_plot::{
    GridMark, HLine, Legend, LineStyle, Plot, PlotPoint, PlotPoints, log_grid_spacer, uniform_grid_spacer,
};

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
    name: String,
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
        Self { name, color, points: all, runs }
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

/// Draws one chart over the times of `range`, in seconds since 1970. The charts of one
/// `group` share their time axis and their cursor; the range moves with the clock, so a
/// chart has no zoom and no drag. `number` tells the charts of a group apart.
pub(crate) fn show(
    ui: &mut Ui,
    palette: &Palette,
    clock: &Clock,
    (group, number): (Id, usize),
    range: &RangeInclusive<f64>,
    chart: &Chart<'_>,
) {
    let id = group.with(number);
    let label = |mark: GridMark, _: &RangeInclusive<f64>| {
        let time = time_of(mark.value);
        if mark.step_size < 60.0 { clock.time_of_day(time) } else { clock.hour_minute(time) }
    };
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
        .y_grid_spacer(log_grid_spacer(if chart.binary { 2 } else { 10 }))
        .x_grid_spacer(uniform_grid_spacer(|input| time::steps(input.base_step_size)))
        .x_axis_formatter(label)
        .y_axis_formatter(|mark, _| (chart.text)(mark.value))
        .legend(Legend::default().follow_insertion_order(true));
    let response = plot.show(ui, |plot| {
        plot.set_plot_bounds_x(range.clone());
        for (number, line) in chart.lines.iter().enumerate() {
            // The parts of a line share its identity, so the legend hides and shows them together.
            let id = id.with(number);
            for run in &line.runs {
                let points = PlotPoints::Borrowed(line.points.get(run.clone()).unwrap_or_default());
                plot.line(egui_plot::Line::new(line.name.as_str(), points).color(line.color).width(1.5).id(id));
            }
        }
        for mark in chart.marks {
            plot.hline(HLine::new(mark.name.as_str(), mark.value).color(mark.color).style(LineStyle::dashed_dense()));
        }
    });
    widgets::name(&response.response, &format!("{} chart", chart.title));
    if let Some(pointer) = response.response.hover_pos() {
        let x = response.transform.value_from_position(pointer).x;
        response.response.on_hover_ui_at_pointer(|ui| tooltip(ui, palette, clock, chart, x));
    }
}

/// The value of each line and each mark at the time of the pointer: `egui_plot` gives the
/// value of the nearest line only.
fn tooltip(ui: &mut Ui, palette: &Palette, clock: &Clock, chart: &Chart<'_>, x: f64) {
    ui.label(RichText::new(format!("{} at {}", chart.title, clock.time_of_day(time_of(x)))).strong());
    for line in chart.lines {
        let value = line.at(x, chart.reach).map_or_else(|| "no sample".to_owned(), |value| (chart.text)(value));
        ui.horizontal(|ui| {
            dot(ui, line.color);
            ui.label(format!("{}: {value}", line.name));
        });
    }
    for mark in chart.marks {
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

    #[test]
    fn a_time_of_the_axis_that_is_not_a_time_is_1970() {
        assert_eq!((time_of(-5.0), time_of(f64::NAN)), (SystemTime::UNIX_EPOCH, SystemTime::UNIX_EPOCH));
        assert_eq!(time_of(90.5), SystemTime::UNIX_EPOCH + Duration::from_millis(90_500));
    }
}
