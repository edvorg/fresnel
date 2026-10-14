//! The outage with paused time. Each time here is exact.

use std::cell::RefCell;

use tokio::time::{sleep, timeout};

use super::*;
use crate::channel::ChannelSettings;
use crate::signal::Cause;

const SECOND: Duration = Duration::from_secs(1);

fn outage(signal: &Signal) -> Outage {
    Outage::new(OutageSettings::default(), signal, Changed::new())
}

/// An outage that started `ago` before now.
async fn outage_since(signal: &Signal, ago: Duration) -> Outage {
    let outage = outage(signal);
    outage.failed(Instant::now());
    sleep(ago).await;
    outage
}

/// One channel in an outage with no end: each connect is held until its limit, 10 s for a
/// race and 3 s for one attempt. Gives the start and the effort of each connect.
async fn held_channel(outage: &Outage, signal: &Signal, end: Instant) -> Vec<(Instant, Effort)> {
    let (mut listener, backoff) = (signal.listen(), ChannelSettings::for_watches().backoff);
    let (mut connects, mut effort) = (Vec::new(), Effort::Race);
    for failures in 1.. {
        let started = Instant::now();
        if started >= end {
            break;
        }
        connects.push((started, effort));
        sleep(if effort == Effort::Race { 10 * SECOND } else { 3 * SECOND }).await;
        outage.failed(started);
        effort = outage.pause(&mut listener, backoff.pause(failures, 0), |_| {}).await;
    }
    connects
}

#[tokio::test(start_paused = true)]
async fn after_one_minute_the_channels_take_turns_with_one_attempt_each_5_seconds() {
    let signal = Signal::new();
    let outage = outage(&signal);
    let start = Instant::now();
    let end = start + 600 * SECOND;
    let (one, two, three) = tokio::join!(
        held_channel(&outage, &signal, end),
        held_channel(&outage, &signal, end),
        held_channel(&outage, &signal, end),
    );
    let mut connects: Vec<_> = one.into_iter().chain(two).chain(three).collect();
    connects.sort_by_key(|(started, _)| *started);

    let paced = start + 60 * SECOND;
    let (first, later): (Vec<_>, Vec<_>) = connects.iter().partition(|(started, _)| *started < paced);
    assert!(first.iter().all(|(_, effort)| *effort == Effort::Race), "the first minute keeps the full races");
    assert!(later.iter().all(|(_, effort)| *effort == Effort::Single), "after it, each connect has one attempt");
    for pair in later.windows(2) {
        let [(one, _), (two, _)] = pair else { continue };
        let gap = two.duration_since(*one);
        assert!((4 * SECOND..=6 * SECOND).contains(&gap), "one turn each 5 s with jitter, not {gap:?}");
    }
    // 540 s with one attempt each 4 to 6 s, in place of a race of up to 16 attempts each 13 s.
    assert!((90..=136).contains(&later.len()), "{} attempts after the first minute", later.len());
}

#[tokio::test(start_paused = true)]
async fn a_signal_ends_each_wait_for_a_turn_and_the_turns_start_again() {
    let signal = Signal::new();
    let outage = outage_since(&signal, 70 * SECOND).await;
    let (mut first, mut second, mut third) = (signal.listen(), signal.listen(), signal.listen());
    assert_eq!(outage.pause(&mut first, 3 * SECOND, |_| {}).await, Effort::Single, "the first turn is now");

    let started = Instant::now();
    let shown = RefCell::new(Vec::new());
    let show = |wait| shown.borrow_mut().push(wait);
    let raise = async {
        sleep(SECOND).await;
        signal.raise(Cause::NetworkChange);
    };
    let (two, three, ()) =
        tokio::join!(outage.pause(&mut second, 3 * SECOND, show), outage.pause(&mut third, 3 * SECOND, show), raise);
    assert_eq!((two, three, started.elapsed()), (Effort::Race, Effort::Race, SECOND));
    let shown = shown.into_inner();
    assert!(shown.iter().all(|wait| (4 * SECOND..=12 * SECOND).contains(wait)), "two later turns: {shown:?}");

    // The race after the signal tested the path. No turn from before the signal holds
    // the next one back: it comes one spacing later, not three.
    let mut fresh = signal.listen();
    let started = Instant::now();
    assert_eq!(outage.pause(&mut fresh, 3 * SECOND, |_| {}).await, Effort::Single);
    assert!((4 * SECOND..=6 * SECOND).contains(&started.elapsed()), "{:?}", started.elapsed());
}

#[tokio::test(start_paused = true)]
async fn a_connect_that_works_ends_the_outage_and_each_wait() {
    let signal = Signal::new();
    let outage = outage_since(&signal, 70 * SECOND).await;
    let (mut first, mut second) = (signal.listen(), signal.listen());
    assert_eq!(outage.pause(&mut first, 3 * SECOND, |_| {}).await, Effort::Single);

    let started = Instant::now();
    let works = async {
        sleep(SECOND).await;
        outage.worked();
    };
    let (effort, ()) = tokio::join!(outage.pause(&mut second, 3 * SECOND, |_| {}), works);
    assert_eq!((effort, started.elapsed(), outage.status()), (Effort::Race, SECOND, None));

    // A new outage starts with its first part again: the pause of the channel itself.
    outage.failed(Instant::now());
    let started = Instant::now();
    assert_eq!(outage.pause(&mut second, 3 * SECOND, |_| {}).await, Effort::Race);
    assert_eq!(started.elapsed(), 3 * SECOND);
}

#[tokio::test(start_paused = true)]
async fn the_status_tells_the_start_and_the_turn_and_each_change_tells_the_ui() {
    let (signal, changed) = (Signal::new(), Changed::new());
    let outage = Outage::new(OutageSettings::default(), &signal, changed.clone());
    let mut changes = changed.watch();
    assert_eq!(outage.status(), None);

    let started = Instant::now();
    sleep(2 * SECOND).await;
    outage.failed(started);
    let after = SystemTime::now();
    timeout(SECOND, changes.next()).await.expect("the start of the outage is a change");
    let status = outage.status().expect("an outage");
    let age = after.duration_since(status.since).expect("a start in the past");
    assert!(age >= 2 * SECOND, "the start of the connect, not of its failure: {age:?}");
    assert_eq!(status.turn, None);

    outage.failed(Instant::now());
    assert_eq!(outage.status().expect("an outage").since, status.since, "only the first failure starts it");
    sleep(58 * SECOND).await;
    assert_eq!(outage.status().expect("an outage").turn, Some(5 * SECOND));

    outage.worked();
    timeout(SECOND, changes.next()).await.expect("the end of the outage is a change");
    assert_eq!(outage.status(), None);
}

#[test]
fn a_spacing_is_5_seconds_with_20_percent_of_jitter_and_extreme_values_do_not_panic() {
    let settings = OutageSettings::default();
    let spacings: std::collections::BTreeSet<_> = (0..500).map(|sample| settings.spacing(sample)).collect();
    assert_eq!(spacings.len(), 41, "each percent step occurs");
    assert_eq!(spacings.first().copied().zip(spacings.last().copied()), Some((4 * SECOND, 6 * SECOND)));

    let extreme = OutageSettings { turn: Duration::MAX, jitter_percent: u8::MAX, ..settings };
    assert!(extreme.spacing(u32::MAX) > Duration::ZERO);
    assert_eq!(OutageSettings { jitter_percent: 100, ..settings }.spacing(0), Duration::ZERO);
}
