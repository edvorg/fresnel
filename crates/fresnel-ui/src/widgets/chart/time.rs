//! The marks of a time axis: on full minutes and hours of the clock, not on the decimal
//! steps of a number axis (research 11 section 7.4: `egui_plot` has no time axis).

/// The steps of the grid lines, in seconds. Each one divides the next, so each line of a
/// large step is a line of each smaller step too.
const STEPS: [f64; 8] = [15.0, 60.0, 300.0, 900.0, 3_600.0, 10_800.0, 21_600.0, DAY];
const DAY: f64 = 86_400.0;

/// The three steps of the grid of a time axis, from thin lines to thick ones: the first
/// step of the table that is at least `base` seconds, the smallest distance of two lines
/// of the plot, and the two steps after it.
pub(super) fn steps(base: f64) -> [f64; 3] {
    let first = STEPS.iter().position(|step| *step >= base).unwrap_or(STEPS.len());
    [0, 1, 2].map(|offset| STEPS.get(first + offset).copied().unwrap_or(DAY))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lines_of_a_time_axis_are_on_full_minutes_and_hours() {
        // 5 minutes, 30 minutes and 24 hours over 400 points, with a line each 8 points at most.
        let base = |range: f64| range / 400.0 * 8.0;
        assert_eq!(steps(base(300.0)), [15.0, 60.0, 300.0]);
        assert_eq!(steps(base(1_800.0)), [60.0, 300.0, 900.0]);
        assert_eq!(steps(base(86_400.0)), [3_600.0, 10_800.0, 21_600.0]);
        assert_eq!(steps(1e9), [86_400.0; 3], "a range of years keeps the largest step");
        assert_eq!(steps(0.0), [15.0, 60.0, 300.0]);
    }
}
