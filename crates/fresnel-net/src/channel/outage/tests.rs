//! The outage with paused time. Each time here is exact.

use std::cell::RefCell;

use futures_util::future::join_all;
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
/// race and 3 s for one attempt. A round starts the connect again as a race, as the driver
/// does. Gives the start and the effort of each connect.
async fn held_channel(outage: &Outage, signal: &Signal, end: Instant) -> Vec<(Instant, Effort)> {
    let (mut listener, backoff) = (signal.listen(), ChannelSettings::for_watches().backoff);
    let (mut connects, mut effort) = (Vec::new(), Effort::Race);
    for failures in 1.. {
        let started = Instant::now();
        if started >= end {
            break;
        }
        connects.push((started, effort));
        let held = if effort == Effort::Race { 10 * SECOND } else { 3 * SECOND };
        effort = tokio::select! {
            () = sleep(held) => {
                outage.failed(started);
                outage.pause(&mut listener, backoff.pause(failures, 0), |_| {}).await
            }
            () = outage.round(&mut listener) => Effort::Race,
        };
    }
    connects
}

/// The connects of 3 held channels in an outage of `length`, with a raise at each instant
/// of `raises` after the start of the outage. Gives the start of each race of the channels
/// after `from`, in sequence: a round starts a race of each channel at the same instant.
async fn races_with(raises: Vec<Duration>, length: Duration, from: Duration) -> Vec<Duration> {
    let signal = Signal::new();
    let outage = outage(&signal);
    let start = Instant::now();
    let raise = async {
        for at in raises {
            sleep((start + at).saturating_duration_since(Instant::now())).await;
            signal.raise(Cause::NetworkChange);
        }
    };
    let channels = join_all((0..3).map(|_| held_channel(&outage, &signal, start + length)));
    let (connects, ()) = tokio::join!(channels, raise);
    let races = connects.into_iter().flatten().filter(|(_, effort)| *effort == Effort::Race);
    let mut races: Vec<_> = races.map(|(started, _)| started - start).filter(|after| *after >= from).collect();
    races.sort();
    races
}

/// The instants of the rounds: the races of one round start at the same instant.
fn rounds(races: &[Duration]) -> Vec<Duration> {
    let mut rounds = races.to_vec();
    rounds.dedup();
    rounds
}

/// The connects of `count` held channels of one cluster in an outage of 10 minutes, in the
/// sequence of their starts, with the start of the outage.
async fn ten_minutes_of(count: usize) -> (Instant, Vec<(Instant, Effort)>) {
    let signal = Signal::new();
    let outage = outage(&signal);
    let start = Instant::now();
    let channels = (0..count).map(|_| held_channel(&outage, &signal, start + 600 * SECOND));
    let mut connects: Vec<_> = join_all(channels).await.into_iter().flatten().collect();
    connects.sort_by_key(|(started, _)| *started);
    (start, connects)
}

/// A group opens up to 3 channels, so a cluster can have 9. The cost of the cluster and the
/// longest time with no attempt do not grow with the count of its channels.
#[tokio::test(start_paused = true)]
async fn after_one_minute_the_channels_take_turns_with_one_attempt_each_5_seconds() {
    for count in [3, 9] {
        let (start, connects) = ten_minutes_of(count).await;
        let paced = start + 60 * SECOND;
        let (first, later): (Vec<_>, Vec<_>) = connects.iter().partition(|(started, _)| *started < paced);
        assert!(first.iter().all(|(_, effort)| *effort == Effort::Race), "the first minute keeps the full races");
        assert!(later.iter().all(|(_, effort)| *effort == Effort::Single), "after it, each connect has one attempt");
        for pair in later.windows(2) {
            let [(one, _), (two, _)] = pair else { continue };
            let gap = two.duration_since(*one);
            assert!((4 * SECOND..=6 * SECOND).contains(&gap), "{count} channels: one turn each 5 s, not {gap:?}");
        }
        // 540 s with one attempt each 4 to 6 s, in place of a race of up to 16 attempts each 13 s.
        assert!((90..=136).contains(&later.len()), "{count} channels: {} attempts after the first minute", later.len());
    }
}

/// Channel A starts a connect that is held, and channel B connects in that time. The path
/// worked after A started, so the failure of A tells nothing new.
#[tokio::test(start_paused = true)]
async fn a_connect_that_started_before_the_last_one_that_worked_starts_no_outage() {
    let signal = Signal::new();
    let outage = outage(&signal);
    let held = Instant::now();
    sleep(2 * SECOND).await;
    let worked = Instant::now();
    outage.worked();
    sleep(SECOND).await;
    outage.failed(held);
    assert_eq!(outage.status(), None, "the path worked after that connect started");

    // A connect that started at the same instant, or later, tells of a new fault.
    outage.failed(worked);
    let since = outage.status().expect("an outage").since;
    let age = SystemTime::now().duration_since(since).expect("a start in the past");
    assert_eq!(age.as_secs(), 1, "the start of that connect: {age:?}");
}

/// A setting can be extreme. A turn of `Duration::MAX` gives a long wait: a sum of two times
/// that passes the end of the clock would panic and stop the task of the channel each turn.
#[tokio::test(start_paused = true)]
async fn a_turn_longer_than_the_clock_can_hold_waits_and_does_not_panic() {
    let signal = Signal::new();
    let settings = OutageSettings { turn: Duration::MAX, ..OutageSettings::default() };
    let outage = Outage::new(settings, &signal, Changed::new());
    outage.failed(Instant::now());
    sleep(70 * SECOND).await;
    let (mut first, mut second) = (signal.listen(), signal.listen());
    assert_eq!(outage.pause(&mut first, 3 * SECOND, |_| {}).await, Effort::Single, "the first turn is now");

    let shown = RefCell::new(Vec::new());
    let next = outage.pause(&mut second, 3 * SECOND, |wait| shown.borrow_mut().push(wait));
    assert!(timeout(3_600 * SECOND, next).await.is_err(), "the next turn is far away");
    assert!(shown.into_inner().iter().all(|wait| *wait > 3_600 * SECOND), "the state tells a long wait");
}

/// The detectors can tell a change each 2 s. From minute 2 to minute 10 the channels take
/// turns, so each race comes from a round, and a round comes 10 s after the one before at
/// the earliest: 48 rounds in 480 s, in place of 240.
#[tokio::test(start_paused = true)]
async fn a_raise_each_2_seconds_in_a_long_outage_starts_one_round_each_10_seconds_at_most() {
    let raises = (60..300).map(|step| step * 2 * SECOND).collect();
    let races = races_with(raises, 600 * SECOND, 120 * SECOND).await;
    let rounds = rounds(&races);
    assert_eq!(rounds.first(), Some(&(120 * SECOND)), "the first raise starts a round at once");
    for pair in rounds.windows(2) {
        let [one, two] = pair else { continue };
        assert!(*two - *one >= 10 * SECOND, "{one:?} and {two:?}: two rounds in less than 10 s");
    }
    assert!(rounds.len() <= 48, "{} rounds in 480 s", rounds.len());
    assert!(races.len() <= 3 * rounds.len(), "one race of each channel in a round: {races:?}");
}

/// The first raise after a calm time starts a round at once. A storm of raises from 120 s
/// to 148 s gives rounds at 120, 130 and 140 s. Then a raise at 158 s and one at 170 s each
/// start a round, and one at 175 s does not.
#[tokio::test(start_paused = true)]
async fn a_raise_after_a_calm_time_starts_a_round_at_once() {
    let storm = (60..75).map(|step| step * 2 * SECOND);
    let raises = storm.chain([158 * SECOND, 170 * SECOND, 175 * SECOND]).collect();
    let races = races_with(raises, 300 * SECOND, 120 * SECOND).await;
    let rounds = rounds(&races);
    let storm = [120, 130, 140].map(|at| at * SECOND);
    assert_eq!(rounds, [storm.as_slice(), &[158 * SECOND, 170 * SECOND]].concat(), "{races:?}");
    let at_170 = races.iter().filter(|after| **after == 170 * SECOND).count();
    assert_eq!(at_170, 3, "each channel starts a race at once: {races:?}");
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
