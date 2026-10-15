//! The time that the server writes before each line with `timestamps=true`.
//!
//! The kubelet writes the layout `2006-01-02T15:04:05.000000000Z07:00` and one space; the
//! offset can be other than `Z` (research 05, review claim 19). The app always asks for the
//! time: it sorts the lines of many streams with it, and a new stream starts from it.

use k8s_openapi::jiff::Timestamp;

/// Nanoseconds in one second.
pub(crate) const NANOS: i64 = 1_000_000_000;
/// The longest time of the layout, with an offset such as `+05:30`.
const LONGEST: usize = 35;

/// The time of a line in nanoseconds since 1970 and the text after it. `None` for a line
/// that does not start with a time.
pub(super) fn split(line: &[u8]) -> Option<(i64, &[u8])> {
    let space = line.iter().take(LONGEST + 1).position(|byte| *byte == b' ')?;
    let (head, rest) = line.split_at(space);
    let time: Timestamp = std::str::from_utf8(head).ok()?.parse().ok()?;
    let nanos = i64::try_from(time.as_nanosecond()).ok()?;
    Some((nanos, rest.get(1..)?))
}

/// The full second at or before a time, for the parameter `sinceTime` of a new stream.
///
/// kube rounds `sinceTime` to the nearest second, and the kubelet drops each line before
/// it. With its fraction, the time of the last line could lose the lines of up to half a
/// second (research 08, section 6.1).
pub(crate) fn second_of(nanos: i64) -> Timestamp {
    // Each second of an i64 of nanoseconds is in the range of jiff. The first time of jiff
    // would read the full log, which loses no line.
    Timestamp::from_second(nanos.div_euclid(NANOS)).unwrap_or(Timestamp::MIN)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_time_and_the_text_of_a_line_come_apart_with_each_offset() {
        let (time, text) = split(b"2026-10-12T20:00:00.987654321Z hello world").expect("a time");
        assert_eq!((time % NANOS, text), (987_654_321, &b"hello world"[..]));
        assert_eq!(second_of(time).to_string(), "2026-10-12T20:00:00Z");

        let (east, _) = split(b"2026-10-12T22:00:00.987654321+02:00 x").expect("a time with an offset");
        assert_eq!(east, time);
        assert_eq!(split(b"2026-10-12T20:00:00.5Z "), Some((time - 487_654_321, &b""[..])));
    }

    #[test]
    fn a_line_with_no_time_gives_none() {
        for line in [&b"hello world"[..], b"", b"2026-10-12T20:00:00Z", b"2026-13-12T20:00:00Z x", b"\xff\xfe x"] {
            assert_eq!(split(line), None, "{}", String::from_utf8_lossy(line));
        }
    }

    #[test]
    fn the_second_of_a_time_before_1970_is_the_second_at_or_before_it() {
        assert_eq!(second_of(-1).to_string(), "1969-12-31T23:59:59Z");
        assert_eq!(second_of(0).to_string(), "1970-01-01T00:00:00Z");
    }
}
