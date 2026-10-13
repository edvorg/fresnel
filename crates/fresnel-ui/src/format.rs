//! Texts for counts, time spans and clock times. Each view uses these, so that one value
//! reads the same in each place.

use std::borrow::Cow;
use std::time::{Duration, SystemTime};

use fresnel_core::net::WallClock;
use jiff::Timestamp;
use jiff::tz::TimeZone;

/// "50,000": a count with separators.
pub(crate) fn thousands(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// "1 error", "3 errors": a count with its noun. The noun gets an "s" for its plural.
pub(crate) fn count(value: u64, noun: &str) -> String {
    let plural = if value == 1 { "" } else { "s" };
    format!("{} {noun}{plural}", thousands(value))
}

/// "12 ms", "1.4 s": a short time, for a round trip.
pub(crate) fn short_time(time: Duration) -> String {
    if time < Duration::from_secs(1) {
        format!("{} ms", time.as_millis())
    } else {
        format!("{:.1} s", time.as_secs_f64())
    }
}

/// "45 s", "3 min 5 s", "2 h 10 min", "3 d 4 h": a time span with its two largest units.
pub(crate) fn span(time: Duration) -> String {
    let seconds = time.as_secs();
    let (days, hours, minutes) = (seconds / 86_400, seconds % 86_400 / 3600, seconds % 3600 / 60);
    if days > 0 {
        format!("{days} d {hours} h")
    } else if hours > 0 {
        format!("{hours} h {minutes} min")
    } else if minutes > 0 {
        format!("{minutes} min {} s", seconds % 60)
    } else {
        format!("{seconds} s")
    }
}

/// The first `limit` characters of a text of a cluster, with a mark for the part that
/// does not show. The API gives most texts no limit of their length, and the layout of a
/// text of 1 MB takes longer than a frame.
pub(crate) fn short(text: &str, limit: usize) -> Cow<'_, str> {
    match text.char_indices().nth(limit) {
        Some((end, _)) => Cow::Owned(format!("{}…", text.get(..end).unwrap_or_default())),
        None => Cow::Borrowed(text),
    }
}

/// The wall clock and the time zone of the views. A view test gives a fixed time, so that
/// its texts do not change from run to run.
#[derive(Clone, Debug)]
pub struct Clock {
    now: WallClock,
    zone: TimeZone,
}

impl Clock {
    /// The clock and the time zone of the system.
    pub fn system() -> Self {
        Self { now: WallClock::system(), zone: TimeZone::system() }
    }

    /// A clock that always shows `now`, in UTC.
    pub fn fixed(now: SystemTime) -> Self {
        Self { now: WallClock::new(move || now), zone: TimeZone::UTC }
    }

    /// The time from now to `later`. Zero when `later` is in the past.
    pub(crate) fn until(&self, later: SystemTime) -> Duration {
        later.duration_since(self.now.now()).unwrap_or_default()
    }

    /// The time from `earlier` to now. Zero when `earlier` is in the future.
    pub(crate) fn since(&self, earlier: SystemTime) -> Duration {
        self.now.now().duration_since(earlier).unwrap_or_default()
    }

    /// "14:03:07": the time of day in the time zone of the clock.
    pub(crate) fn time_of_day(&self, time: SystemTime) -> String {
        self.text(time, "%H:%M:%S").unwrap_or_else(|| "--:--:--".to_owned())
    }

    /// "2027-01-15 14:03:07": the day and the time of day in the time zone of the clock.
    pub(crate) fn date_time(&self, time: SystemTime) -> String {
        self.text(time, "%Y-%m-%d %H:%M:%S").unwrap_or_else(|| "no date".to_owned())
    }

    /// `None` for a time that the calendar cannot show, for example before the year -9999.
    fn text(&self, time: SystemTime, form: &str) -> Option<String> {
        let timestamp = Timestamp::try_from(time).ok()?;
        Some(timestamp.to_zoned(self.zone.clone()).strftime(form).to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECOND: Duration = Duration::from_secs(1);

    #[test]
    fn a_count_has_separators_and_the_noun_follows_the_number() {
        assert_eq!((thousands(0), thousands(999), thousands(50_000)), ("0".into(), "999".into(), "50,000".into()));
        assert_eq!(thousands(1_234_567), "1,234,567");
        assert_eq!(
            (count(0, "error"), count(1, "error"), count(1200, "warning")),
            ("0 errors".into(), "1 error".into(), "1,200 warnings".into())
        );
    }

    #[test]
    fn a_span_shows_its_two_largest_units() {
        let texts: Vec<String> = [0, 59, 60, 185, 3600, 7800, 86_400, 273_600].map(|s| span(s * SECOND)).to_vec();
        assert_eq!(texts, ["0 s", "59 s", "1 min 0 s", "3 min 5 s", "1 h 0 min", "2 h 10 min", "1 d 0 h", "3 d 4 h"]);
        assert_eq!(span(Duration::from_millis(1999)), "1 s");
        assert_eq!(
            (short_time(Duration::from_millis(12)), short_time(Duration::from_millis(1440))),
            ("12 ms".into(), "1.4 s".into())
        );
    }

    #[test]
    fn a_long_text_shows_its_start_with_a_mark_and_a_cut_is_between_two_characters() {
        assert_eq!(short("Running", 7), "Running");
        assert_eq!(short("CrashLoopBackOff", 5), "Crash…");
        assert_eq!(short("größer", 3), "grö…", "a limit counts characters, not bytes");
        assert_eq!((short("", 0), short("a", 0)), ("".into(), "…".into()));
    }

    #[test]
    fn the_clock_gives_the_time_of_day_and_the_distance_to_a_time() {
        let now = SystemTime::UNIX_EPOCH + 1_800_000_000 * SECOND;
        let clock = Clock::fixed(now);
        assert_eq!(clock.time_of_day(now), "08:00:00");
        assert_eq!(clock.time_of_day(now + 3725 * SECOND), "09:02:05");
        assert_eq!(clock.date_time(now + 3725 * SECOND), "2027-01-15 09:02:05");
        assert_eq!((clock.until(now + 5 * SECOND), clock.until(now - 5 * SECOND)), (5 * SECOND, Duration::ZERO));
        assert_eq!((clock.since(now - 7 * SECOND), clock.since(now + 7 * SECOND)), (7 * SECOND, Duration::ZERO));
    }
}
