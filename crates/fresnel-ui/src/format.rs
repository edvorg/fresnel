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

/// "6.1m", "250m", "1.25": CPU from nanocores in the form of `kubectl top`, millicores
/// below one core and cores from one core on.
pub(crate) fn cpu(nanocores: u64) -> String {
    let millicores = nanocores as f64 / 1e6;
    match millicores {
        _ if millicores.round() >= 1000.0 => trimmed(millicores / 1000.0),
        _ if millicores >= 9.95 => format!("{millicores:.0}m"),
        _ if nanocores == 0 => "0m".to_owned(),
        _ if millicores < 0.1 => "<0.1m".to_owned(),
        _ => format!("{millicores:.1}m"),
    }
}

/// "512Ki", "128Mi", "1.5Gi": memory from bytes in the units of the API.
pub(crate) fn memory(bytes: u64) -> String {
    const KI: f64 = 1024.0;
    let value = bytes as f64;
    match value {
        _ if (value / KI / KI).round() >= KI => format!("{}Gi", trimmed(value / KI / KI / KI)),
        _ if value >= KI * KI => format!("{:.0}Mi", value / KI / KI),
        _ if value >= KI => format!("{:.0}Ki", value / KI),
        _ => bytes.to_string(),
    }
}

/// A number with two decimals at most and no zero at the end: "1.25", "1.5", "2".
fn trimmed(value: f64) -> String {
    let text = format!("{value:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
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

/// The last `limit` characters of a text, after a mark for the part that does not show: the
/// names of the pods of one owner differ at their end.
pub(crate) fn short_start(text: &str, limit: usize) -> Cow<'_, str> {
    match text.chars().count().checked_sub(limit).filter(|cut| *cut > 0) {
        Some(cut) => {
            let start = text.char_indices().nth(cut).map_or(text.len(), |(start, _)| start);
            Cow::Owned(format!("…{}", text.get(start..).unwrap_or_default()))
        }
        None => Cow::Borrowed(text),
    }
}

/// The short names of the pods of one owner: each name without the start that all of them
/// share, up to a dash. The pods of one ReplicaSet are `gn8jp` and `k8q55`; in a rollout they
/// keep the hash of their ReplicaSet, as `7d9c5b8f6-gn8jp`, and the pods of an Indexed Job
/// their index, as `0-x7k2p`. The view names the owner, and a legend or a log line with the
/// full names has no room for more. The number of a pod of a StatefulSet alone would read
/// as a value, so such a pod keeps its full name.
pub(crate) struct PodNames {
    shared: String,
}

impl PodNames {
    pub(crate) fn of<'a>(pods: impl IntoIterator<Item = &'a str>) -> Self {
        let mut pods = pods.into_iter();
        let first = pods.next().unwrap_or_default();
        let mut same = first.len();
        for pod in pods {
            same = first.bytes().zip(pod.bytes()).take(same).take_while(|(a, b)| a == b).count();
        }
        let dash = first.get(..same).and_then(|start| start.rfind('-'));
        Self { shared: dash.and_then(|dash| first.get(..=dash)).unwrap_or_default().to_owned() }
    }

    /// The short name of a pod of the owner. Any other name stays as it is.
    pub(crate) fn short<'a>(&self, pod: &'a str) -> &'a str {
        let rest = pod.strip_prefix(self.shared.as_str());
        rest.filter(|rest| !rest.is_empty() && !rest.bytes().all(|byte| byte.is_ascii_digit())).unwrap_or(pod)
    }
}

/// The text of a time of day that the calendar cannot show.
const NO_TIME: &str = "--:--:--";

/// The wall clock and the time zone of the views. A view test gives a fixed time, so that
/// its texts do not change from run to run.
#[derive(Clone, Debug)]
pub struct Clock {
    now: WallClock,
    zone: TimeZone,
    /// True for the form "02:03:07 PM" of the settings.
    twelve: bool,
}

impl Clock {
    /// The clock and the time zone of the system.
    pub fn system() -> Self {
        Self { now: WallClock::system(), zone: TimeZone::system(), twelve: false }
    }

    /// A clock that always shows `now`, in UTC.
    pub fn fixed(now: SystemTime) -> Self {
        Self { now: WallClock::new(move || now), zone: TimeZone::UTC, twelve: false }
    }

    /// Takes the time zone and the form of a time of day of the settings.
    pub(crate) fn set(&mut self, zone: TimeZone, twelve: bool) {
        (self.zone, self.twelve) = (zone, twelve);
    }

    /// The form of a time of day of the strftime of jiff, with or without seconds.
    fn form(&self, seconds: &'static str) -> String {
        if self.twelve { format!("%I:%M{seconds} %p") } else { format!("%H:%M{seconds}") }
    }

    /// The time of now.
    pub(crate) fn now(&self) -> SystemTime {
        self.now.now()
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
        self.text(time, &self.form(":%S")).unwrap_or_else(|| NO_TIME.to_owned())
    }

    /// "14:03": the hour and the minute in the time zone of the clock, for the marks of a
    /// time axis.
    pub(crate) fn hour_minute(&self, time: SystemTime) -> String {
        self.text(time, &self.form("")).unwrap_or_else(|| NO_TIME.to_owned())
    }

    /// The time of day `wait` after now. A wait from extreme settings can be past the end of
    /// the clock, and a plain sum would then stop the frame.
    pub(crate) fn time_of_day_after(&self, wait: Duration) -> String {
        self.now.now().checked_add(wait).map_or_else(|| NO_TIME.to_owned(), |time| self.time_of_day(time))
    }

    /// "14:03:07.123": the time of a log line in nanoseconds since 1970, with its milliseconds,
    /// in the time zone of the clock. The calendar has each time of an `i64`.
    pub(crate) fn log_time(&self, nanos: i64) -> String {
        match Timestamp::from_nanosecond(i128::from(nanos)) {
            Ok(time) => time.to_zoned(self.zone.clone()).strftime(&self.form(":%S%.3f")).to_string(),
            Err(_) => format!("{NO_TIME}.---"),
        }
    }

    /// "2027-01-15 14:03:07": the day and the time of day in the time zone of the clock.
    pub(crate) fn date_time(&self, time: SystemTime) -> String {
        self.text(time, &format!("%Y-%m-%d {}", self.form(":%S"))).unwrap_or_else(|| "no date".to_owned())
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
    fn cpu_and_memory_have_the_units_of_kubectl_top() {
        let cpu: Vec<String> =
            [0, 40_000, 6_082_165, 9_960_000, 250_000_000, 999_600_000, 1_250_000_000, 2_000_000_000].map(cpu).to_vec();
        assert_eq!(cpu, ["0m", "<0.1m", "6.1m", "10m", "250m", "1", "1.25", "2"]);
        let memory: Vec<String> =
            [100, 2048, 22_272 * 1024, 1_073_217_536, 1_610_612_736, 3 << 40].map(memory).to_vec();
        assert_eq!(memory, ["100", "2Ki", "22Mi", "1Gi", "1.5Gi", "3072Gi"]);
    }

    #[test]
    fn a_long_text_shows_its_start_with_a_mark_and_a_cut_is_between_two_characters() {
        assert_eq!(short("Running", 7), "Running");
        assert_eq!(short("CrashLoopBackOff", 5), "Crash…");
        assert_eq!(short("größer", 3), "grö…", "a limit counts characters, not bytes");
        assert_eq!((short("", 0), short("a", 0)), ("".into(), "…".into()));
    }

    #[test]
    fn a_long_text_shows_its_end_with_a_mark_before_it() {
        assert_eq!(short_start("postgres-0", 10), "postgres-0");
        assert_eq!(short_start("elasticsearch-master-0", 15), "…search-master-0");
        assert_eq!(short_start("größer", 3), "…ßer", "a limit counts characters, not bytes");
        assert_eq!((short_start("", 0), short_start("a", 0)), ("".into(), "…".into()));
    }

    #[test]
    fn a_pod_has_the_rest_of_its_name_after_the_start_that_the_pods_of_its_owner_share() {
        let shorts = |pods: &[&str]| {
            let names = PodNames::of(pods.iter().copied());
            pods.iter().map(|pod| names.short(pod).to_owned()).collect::<Vec<String>>()
        };
        let set = ["checkout-api-7d9c5b8f6-gn8jp", "checkout-api-7d9c5b8f6-k8q55"];
        assert_eq!(shorts(&set), ["gn8jp", "k8q55"], "the pods of one ReplicaSet");
        assert_eq!(shorts(&set[..1]), ["gn8jp"], "one pod");
        assert_eq!(
            shorts(&["checkout-api-7d9c5b8f6-gn8jp", "checkout-api-7f6b7c8d9-ab12c"]),
            ["7d9c5b8f6-gn8jp", "7f6b7c8d9-ab12c"],
            "a rollout keeps the hash of each ReplicaSet, though the hashes start alike"
        );
        assert_eq!(shorts(&["train-0-x7k2p", "train-1-q9z4m"]), ["0-x7k2p", "1-q9z4m"], "an Indexed Job");
        assert_eq!(shorts(&["report-29999640-k2x7p"]), ["k2x7p"], "a pod of a Job of a CronJob");
        assert_eq!(
            shorts(&["postgres-0", "postgres-1"]),
            ["postgres-0", "postgres-1"],
            "a number alone reads as a value"
        );
        assert_eq!(shorts(&["web", "api"]), ["web", "api"]);
        let names = PodNames::of(set);
        assert_eq!((names.short("all pods"), PodNames::of([]).short("web")), ("all pods", "web"), "an other name");
    }

    #[test]
    fn the_clock_gives_the_time_of_day_and_the_distance_to_a_time() {
        let now = SystemTime::UNIX_EPOCH + 1_800_000_000 * SECOND;
        let clock = Clock::fixed(now);
        assert_eq!(clock.time_of_day(now), "08:00:00");
        assert_eq!(clock.time_of_day(now + 3725 * SECOND), "09:02:05");
        assert_eq!(clock.date_time(now + 3725 * SECOND), "2027-01-15 09:02:05");
        assert_eq!(clock.hour_minute(now + 3725 * SECOND), "09:02");
        assert_eq!(clock.log_time(1_800_000_000_123_456_789), "08:00:00.123");
        assert_eq!((clock.until(now + 5 * SECOND), clock.until(now - 5 * SECOND)), (5 * SECOND, Duration::ZERO));
        assert_eq!((clock.since(now - 7 * SECOND), clock.since(now + 7 * SECOND)), (7 * SECOND, Duration::ZERO));
        assert_eq!(
            (clock.time_of_day_after(3725 * SECOND), clock.time_of_day_after(Duration::MAX)),
            ("09:02:05".into(), "--:--:--".into())
        );
    }

    #[test]
    fn the_settings_give_the_clock_its_time_zone_and_the_form_of_twelve_hours() {
        let now = SystemTime::UNIX_EPOCH + 1_800_000_000 * SECOND;
        let mut clock = Clock::fixed(now);
        clock.set(TimeZone::get("America/New_York").expect("a time zone"), true);
        assert_eq!(clock.time_of_day(now + 6 * 3600 * SECOND), "09:00:00 AM");
        assert_eq!(clock.time_of_day(now + 12 * 3600 * SECOND), "03:00:00 PM");
        assert_eq!(
            (clock.hour_minute(now), clock.log_time(1_800_000_000_123_456_789)),
            ("03:00 AM".into(), "03:00:00.123 AM".into())
        );
        assert_eq!(clock.date_time(now), "2027-01-15 03:00:00 AM");
    }
}
