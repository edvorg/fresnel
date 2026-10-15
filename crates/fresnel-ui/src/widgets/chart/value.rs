//! The marks of a value axis in decimal units, such as cores of CPU: on 1, 2 and 5 times a
//! power of 10, as on a ruler. On the powers of 10 alone, the axis of a pod with a limit of
//! 1.5 cores read only "0" and "1".

/// `egui_plot` draws a grid line for a step of 8 points or more, the `base` of `steps`, and
/// a label for a step of 20 points or more.
const LABEL_ROOM: f64 = 2.5;

/// The three steps of the grid of a value axis, from thin lines to thick ones. The middle
/// one has the labels: the first of 1, 2 and 5 times a power of 10 with room for a label.
/// Each step divides the next, so a line of a large step is a line of each smaller one too.
pub(super) fn steps(base: f64) -> [f64; 3] {
    if !base.is_finite() || base <= 0.0 {
        return [1.0; 3];
    }
    let wanted = base * LABEL_ROOM;
    let power = 10_f64.powi(wanted.log10().floor() as i32);
    // Each label step with the steps below and above it: 0.2, 1, 5; 1, 2, 10; 1, 5, 10.
    let choices = [(1.0, 5.0, 5.0), (2.0, 2.0, 5.0), (5.0, 5.0, 2.0), (10.0, 5.0, 5.0)];
    let mut chosen = choices.into_iter().map(|(mantissa, below, above)| (mantissa * power, below, above));
    let (label, below, above) = chosen.find(|(step, _, _)| *step >= wanted).unwrap_or((10.0 * power, 5.0, 5.0));
    [label / below, label, label * above]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_labels_of_a_value_axis_are_on_1_2_and_5_times_a_power_of_10() {
        // 1.6 cores and 8 cores over some 125 points, with a grid line each 8 points at most.
        let base = |range: f64| range / 125.0 * 8.0;
        assert_eq!(steps(base(1.7)), [0.1, 0.5, 1.0], "0, 500m, 1, 1.5");
        assert_eq!(steps(base(8.5)), [1.0, 2.0, 10.0], "0, 2, 4, 6, 8");
        assert_eq!(steps(0.3), [0.2, 1.0, 5.0]);
        assert_eq!([steps(0.0), steps(f64::NAN)], [[1.0; 3]; 2], "a range of nothing");
    }
}
