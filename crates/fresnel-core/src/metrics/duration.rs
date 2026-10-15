//! The reader of a Go duration, the form of the `window` of a metrics item: `15.012s`,
//! `1m0.5s`, `500ms` (research 11, review item 6).

use std::time::Duration;

/// The units of Go, in nanoseconds. `µs` has two forms in Unicode, and Go reads both.
const UNITS: [(&str, u128); 9] = [
    ("ns", 1),
    ("us", 1_000),
    ("\u{b5}s", 1_000),
    ("\u{3bc}s", 1_000),
    ("ms", 1_000_000),
    ("s", 1_000_000_000),
    ("m", 60_000_000_000),
    ("h", 3_600_000_000_000),
    ("", 0),
];

/// Reads a duration of Go, as `time.ParseDuration` does: one or more parts of a number and
/// a unit. A part of a nanosecond is cut off. A sign `-` is an error: no window is below zero.
pub(crate) fn parse(text: &str) -> Result<Duration, &'static str> {
    let mut rest = text.strip_prefix('+').unwrap_or(text);
    if rest.starts_with('-') {
        return Err("below zero");
    }
    if rest == "0" {
        return Ok(Duration::ZERO);
    }
    if rest.is_empty() {
        return Err("no number");
    }
    let mut nanos: u128 = 0;
    while !rest.is_empty() {
        let end = rest.find(|c: char| !c.is_ascii_digit() && c != '.').unwrap_or(rest.len());
        let (number, after) = rest.split_at(end);
        let unit_end = after.find(|c: char| c.is_ascii_digit() || c == '.').unwrap_or(after.len());
        let (unit, next) = after.split_at(unit_end);
        let factor = UNITS.iter().find(|(name, _)| *name == unit).map(|(_, factor)| *factor);
        let factor = factor.filter(|factor| *factor > 0).ok_or("no unit or an unknown unit")?;
        nanos = nanos.checked_add(part(number, factor)?).ok_or("too long")?;
        rest = next;
    }
    let seconds = u64::try_from(nanos / 1_000_000_000).map_err(|_| "too long")?;
    // The remainder of a division by 10^9 fits in `u32`.
    Ok(Duration::new(seconds, u32::try_from(nanos % 1_000_000_000).unwrap_or_default()))
}

/// The nanoseconds of one number in a unit of `factor` nanoseconds.
fn part(number: &str, factor: u128) -> Result<u128, &'static str> {
    let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
    if whole.is_empty() && fraction.is_empty() || fraction.contains('.') {
        return Err("no number");
    }
    let read = |digits: &str| if digits.is_empty() { Ok(0) } else { digits.parse::<u128>().map_err(|_| "too long") };
    let whole = read(whole)?.checked_mul(factor).ok_or("too long")?;
    // Go reads at most the digits of the fraction that change the result.
    let fraction = fraction.get(..18).unwrap_or(fraction);
    let scale = 10u128.pow(u32::try_from(fraction.len()).unwrap_or_default());
    let fraction = read(fraction)?.checked_mul(factor).ok_or("too long")? / scale;
    whole.checked_add(fraction).ok_or("too long")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_form_of_go_gives_its_duration() {
        let cases = [
            ("15.012s", Duration::from_millis(15_012)),
            ("1m0.5s", Duration::from_millis(60_500)),
            ("500ms", Duration::from_millis(500)),
            ("1h2m3s", Duration::from_secs(3_723)),
            ("1.5h", Duration::from_secs(5_400)),
            ("2us", Duration::from_micros(2)),
            ("2\u{b5}s", Duration::from_micros(2)),
            ("2\u{3bc}s", Duration::from_micros(2)),
            ("7ns", Duration::from_nanos(7)),
            ("0", Duration::ZERO),
            ("0s", Duration::ZERO),
            ("+5s", Duration::from_secs(5)),
            (".5s", Duration::from_millis(500)),
            ("1.s", Duration::from_secs(1)),
            ("1.0000000001s", Duration::from_secs(1)),
            ("0.1234567891234567891234s", Duration::from_nanos(123_456_789)),
        ];
        for (text, duration) in cases {
            assert_eq!(parse(text), Ok(duration), "{text}");
        }
    }

    #[test]
    fn a_text_that_is_no_duration_gives_its_reason() {
        let cases = [
            ("", "no number"),
            ("-1s", "below zero"),
            ("15", "no unit or an unknown unit"),
            ("15x", "no unit or an unknown unit"),
            ("1d", "no unit or an unknown unit"),
            ("s", "no number"),
            ("1..2s", "no number"),
            ("99999999999999999999999999999999999999999h", "too long"),
        ];
        for (text, reason) in cases {
            assert_eq!(parse(text), Err(reason), "{text:?}");
        }
    }
}
