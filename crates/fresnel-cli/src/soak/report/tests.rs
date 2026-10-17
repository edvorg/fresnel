use std::path::PathBuf;

use super::super::check::Gap;
use super::super::logs::LogCounts;
use super::*;

const MIB_BYTES: usize = 1024 * 1024;

fn report(warm_up: u64) -> Report {
    let options = Options {
        context: "k3s".to_owned(),
        kubeconfig: vec![PathBuf::from("/k")],
        compare: Vec::new(),
        report: PathBuf::from("/r"),
        minutes: 60,
        log: Some(("soak".to_owned(), "logger".to_owned())),
        memory_limit_mib: 16,
        warm_up,
        faults: None,
        forward: None,
    };
    Report::new(&options, "2027-01-15T08:00:00Z".to_owned())
}

/// A minute that passes, with this heap all the minute long.
pub(in super::super) fn minute(number: u64, heap_mib: usize) -> Minute {
    Minute {
        minute: number,
        time: String::new(),
        epoch: 1_800_000_000 + 60 * number,
        slept_seconds: 0,
        outage: false,
        heap_bytes: heap_mib * MIB_BYTES,
        heap_low: heap_mib * MIB_BYTES,
        heap_high: heap_mib * MIB_BYTES,
        memory: Vec::new(),
        records_bytes: 0,
        descriptors: None,
        health: "Live".to_owned(),
        open_connections: 3,
        opened_connections: 3,
        stores_live: 39,
        stores_not_live: Vec::new(),
        checks: 234,
        lag_median_ms: 0,
        lag_max_ms: 12,
        gaps: Vec::new(),
        failed_lists: Default::default(),
        metrics: None,
        log: Some(LogCounts { lines: 120, ..LogCounts::default() }),
        log_sources: Vec::new(),
        warnings: 0,
        errors: 0,
        faults: Vec::new(),
        forward: None,
    }
}

fn gap(without_state: bool) -> Gap {
    let states = vec!["waits for a connection".to_owned()];
    let reason = "ns/a is not in the store".to_owned();
    Gap { kind: "Pod", seconds: 30.0, live_seconds: 14.0, without_state, states, reason, open: false }
}

fn judged(report: &mut Report, minutes: Vec<Minute>) {
    report.minutes.clear();
    minutes.into_iter().for_each(|minute| report.push(minute));
    report.judge();
}

#[test]
fn a_soak_with_visible_gaps_and_a_heap_inside_its_limit_passes() {
    let mut report = report(2);
    let mut minutes = vec![minute(1, 200), minute(2, 10), minute(3, 70), minute(4, 25)];
    minutes[2].gaps.push(gap(false));
    minutes[2].faults.push("a break of the network of 30 s".to_owned());
    minutes[2].log = Some(LogCounts::default());
    judged(&mut report, minutes);
    let verdict = &report.verdict;
    assert!(verdict.passed, "{:?}", verdict.reasons);
    assert_eq!((verdict.gaps, verdict.gaps_without_state, verdict.faults), (1, 0, 1));
    assert_eq!(verdict.log_quiet_minutes, 1, "a break of the network");
    let heap = verdict.heap.as_ref().expect("the heap after the warm-up");
    let records = report.minutes[1].records_bytes;
    assert!(records > 0, "the soak measures its records");
    assert_eq!((heap.base_bytes + records, heap.growth_bytes), (10 * MIB_BYTES, 0), "the floor takes no peak");
    assert!(report.text().contains("gap of Pod: 30.0 s, live and behind 14.0 s (visible)"), "{}", report.text());
}

#[test]
fn a_gap_with_no_visible_state_a_growth_over_the_limit_and_a_lost_line_each_fail_the_soak() {
    let mut report = report(1);
    let mut minutes: Vec<Minute> = (1..=6).map(|number| minute(number, if number < 2 { 10 } else { 27 })).collect();
    minutes[5].gaps.push(gap(true));
    minutes[5].log = Some(LogCounts { lost: 3, unread: 4, ..LogCounts::default() });
    report.problems.push("task soak check Pod failed".to_owned());
    judged(&mut report, minutes);
    assert!(!report.verdict.passed);
    let reasons = [
        "1 gaps: a store showed \"live\" and was behind for more than 20 s",
        "the floor of the heap with no buffer grew by 17.0 MiB after the warm-up, at minute 6; the limit is 16 MiB",
        "3 log lines did not come",
        "4 log lines left the ring of the view before the soak read them",
        "task soak check Pod failed",
    ];
    assert_eq!(report.verdict.reasons, reasons);
    assert!(report.text().starts_with("soak of k3s from 2027-01-15T08:00:00Z: 6 of 60 minutes\nFAIL: 1 gaps"));
}

#[test]
fn a_store_that_does_not_recover_and_a_log_view_that_stops_each_fail_the_soak() {
    let mut report = report(1);
    let mut minutes: Vec<Minute> = (1..=8).map(|number| minute(number, 10)).collect();
    minutes[1].gaps.push(Gap { open: true, ..gap(false) });
    minutes[3..].iter_mut().for_each(|minute| minute.log = Some(LogCounts::default()));
    judged(&mut report, minutes.clone());
    let reasons = [
        "1 gaps: a store was behind for 300 s and did not recover",
        "the log view gave no line for 5 minutes in a row",
    ];
    assert_eq!(report.verdict.reasons, reasons);
    assert_eq!((report.verdict.gaps_stuck, report.verdict.log_quiet_minutes), (1, 5));

    // The lists of the comparison fail for 5 minutes.
    minutes.iter_mut().for_each(|minute| minute.log = Some(LogCounts { lines: 120, ..LogCounts::default() }));
    minutes[1].gaps.clear();
    minutes[2..7].iter_mut().for_each(|minute| minute.checks = 0);
    judged(&mut report, minutes.clone());
    assert_eq!(report.verdict.reasons, ["no check of a store ended for 5 minutes in a row: the lists failed"]);
    minutes[2..7].iter_mut().for_each(|minute| minute.checks = 234);

    // A view with no line at all; then one whose first lines come late, which passes.
    minutes.iter_mut().for_each(|minute| minute.log = Some(LogCounts::default()));
    judged(&mut report, minutes.clone());
    assert_eq!(report.verdict.reasons, ["the log view gave no line"]);
    let lines = LogCounts { lines: 120, ..LogCounts::default() };
    minutes[6..].iter_mut().for_each(|minute| minute.log = Some(lines.clone()));
    judged(&mut report, minutes);
    assert!(report.verdict.passed, "{:?}", report.verdict.reasons);
}

/// The harness tells the outage when it finds it, after the minutes that it covers began.
#[test]
fn checks_lines_and_a_recovery_that_an_outage_of_the_environment_stops_fail_no_soak_and_count_apart() {
    let mut report = report(1);
    let mut minutes: Vec<Minute> = (1..=12).map(|number| minute(number, 10)).collect();
    for minute in &mut minutes[3..10] {
        (minute.checks, minute.log) = (0, Some(LogCounts { lost: 2, unread: 3, ..LogCounts::default() }));
    }
    minutes[4].gaps.push(Gap { open: true, ..gap(false) });
    let (from, to) = (minutes[3].epoch - 30, minutes[9].epoch - 10);
    minutes[6].faults.push(format!("outage from {from}: the API server of k3s does not answer"));
    minutes[9].faults.push(format!("outage to {to}: the API server answers"));
    minutes[11].slept_seconds = 30;
    judged(&mut report, minutes.clone());
    let verdict = &report.verdict;
    assert!(verdict.passed, "{:?}", verdict.reasons);
    let apart = (verdict.outage_minutes, verdict.outage_unchecked_minutes, verdict.outage_gaps_stuck);
    let lines = (verdict.outage_lost_log_lines, verdict.outage_unread_log_lines);
    assert_eq!((apart, lines, verdict.slept_seconds), ((7, 7, 1), (14, 21), 30));
    assert!(report.text().contains("OUTAGE OF THE ENVIRONMENT"), "{}", report.text());

    // The gap rule holds in an outage.
    minutes[5].gaps.push(gap(true));
    judged(&mut report, minutes.clone());
    assert_eq!(report.verdict.reasons, ["1 gaps: a store showed \"live\" and was behind for more than 20 s"]);
    // With no outage, the same minutes fail.
    minutes.iter_mut().for_each(|minute| minute.faults.clear());
    minutes[5].gaps.clear();
    report = self::report(1);
    judged(&mut report, minutes);
    assert_eq!(report.verdict.reasons.len(), 5, "{:?}", report.verdict.reasons);
}

/// Each part above the limit of its buffer is one reason, with the first minute of it.
#[test]
fn each_part_above_its_limit_fails_the_soak_one_time() {
    use super::super::account::Piece;
    let mut report = report(1);
    let piece = |name: &str, items, bytes| Piece { name: name.to_owned(), items, bytes };
    let mut minutes: Vec<Minute> = (1..=4).map(|number| minute(number, 10)).collect();
    minutes[1].memory = vec![piece("k3s: log ring", 10, 2 * MIB_BYTES), piece("k3s: store Event", 501, 0)];
    minutes[2].memory = vec![piece("k3s: log ring", 10, 3 * MIB_BYTES), piece("journal", 1001, 0)];
    judged(&mut report, minutes);
    let reasons: Vec<&str> =
        report.verdict.reasons.iter().map(|reason| reason.split(" bytes").next().unwrap_or("")).collect();
    assert_eq!(
        reasons,
        [
            "a part above the limit of its buffer: k3s: log ring: 10 lines and 2097152",
            "a part above the limit of its buffer: k3s: store Event: 501 Events, the limit is 500",
            "a part above the limit of its buffer: journal: 1001 entries, the limit is 1000",
        ]
    );
}

/// The measure of the records is all the heap that they hold: a record has no free room in
/// its texts, and the free room of their list counts.
#[test]
fn the_records_have_no_free_room_and_the_room_of_their_list_counts() {
    let mut report = report(1);
    let mut first = minute(1, 10);
    first.time = String::with_capacity(4096);
    first.time.push_str("2027-01-15T08:01:00Z");
    report.push(first);
    let time = &report.minutes[0].time;
    assert_eq!(time.capacity(), time.len());
    report.minutes.reserve_exact(9);
    let room = (report.minutes.capacity() - 1) * size_of::<Minute>();
    let copy = Copies.bytes(&mut || drop(black_box(report.minutes.clone())));
    report.push(minute(2, 10));
    assert_eq!(report.minutes[1].records_bytes, copy + room);
}
