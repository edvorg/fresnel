//! The channels of one cluster in a long outage: one attempt for each turn of the cluster.

use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::connect::dial::Io;
use crate::connect::{ConnectStatus, Effort};

/// The three groups of a cluster, on one server and one outage.
fn cluster(server: &Arc<MemoryServer>) -> (Parts, Vec<Group>) {
    on(server.parts())
}

fn on(parts: Parts) -> (Parts, Vec<Group>) {
    let purposes = [Purpose::Watches, Purpose::Requests, Purpose::Logs];
    let groups = purposes.map(|purpose| Group::start(parts.clone(), purpose, settings())).into();
    (parts, groups)
}

/// A path that lets a count of connects through to the server and refuses each other one.
struct Rationed {
    server: Arc<MemoryServer>,
    left: AtomicUsize,
}

impl Dial for Rationed {
    fn dial(&self, protocol: Protocol, effort: Effort) -> BoxFuture<'_, Result<Io, NetError>> {
        let through = self.left.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| left.checked_sub(1)).is_ok();
        // The server reads its mode in the first poll of the connect, in the same task.
        self.server.set_connects(if through { Connects::Work } else { Connects::Refused });
        self.server.dial(protocol, effort)
    }

    fn status(&self) -> ConnectStatus {
        self.server.status()
    }
}

fn each_live(groups: &[Group]) -> bool {
    groups.iter().all(|group| group.status().channels.iter().all(|channel| channel.state == ChannelState::Live))
}

/// The connects that started in `from..to`, after their start.
fn tries_in(server: &MemoryServer, from: Instant, to: Instant) -> Vec<(Duration, Effort)> {
    let tries = server.tries().into_iter().filter(|(started, _)| (from..to).contains(started));
    tries.map(|(started, effort)| (started - from, effort)).collect()
}

#[tokio::test(start_paused = true)]
async fn in_a_long_outage_the_cluster_connects_one_time_each_5_seconds_and_a_connect_ends_each_pause() {
    let server = MemoryServer::new();
    server.set_connects(Connects::Refused);
    let start = Instant::now();
    let (parts, groups) = cluster(&server);
    sleep(600 * SECOND).await;

    let first = tries_in(&server, start, start + 60 * SECOND);
    assert!(first.iter().all(|(_, effort)| *effort == Effort::Race), "the first minute keeps the races");
    assert!(first.len() > 60, "each channel connects after its own pause: {} connects", first.len());
    let later = tries_in(&server, start + 60 * SECOND, Instant::now());
    assert!(later.iter().all(|(_, effort)| *effort == Effort::Single), "then each connect has one attempt");
    for pair in later.windows(2) {
        let [(one, _), (two, _)] = pair else { continue };
        assert!((4 * SECOND..=6 * SECOND).contains(&(*two - *one)), "one turn each 5 s with jitter: {pair:?}");
    }
    let status = parts.outage.status().expect("an outage");
    assert_eq!(status.turn, Some(5 * SECOND));
    let backoff = groups.iter().filter_map(|group| match group.status().channels.remove(0).state {
        ChannelState::Backoff { pause, .. } => Some(pause),
        _ => None,
    });
    assert!(backoff.max().is_some_and(|pause| pause > 6 * SECOND), "a channel shows the time of its own turn");

    // The next turn connects. Each other channel connects at the same instant.
    let back = Instant::now();
    server.set_connects(Connects::Work);
    let mut changes = parts.changed.watch();
    while !each_live(&groups) {
        changes.next().await;
    }
    let tries = tries_in(&server, back, Instant::now() + MS);
    let [(turn, Effort::Single), (one, Effort::Race), (two, Effort::Race)] = tries[..] else {
        panic!("one attempt in its turn, then a race of each other channel: {tries:?}");
    };
    assert!(turn <= 6 * SECOND && (one, two) == (turn, turn), "{tries:?}");
    assert_eq!(parts.outage.status(), None);
}

#[tokio::test(start_paused = true)]
async fn a_signal_in_a_long_outage_starts_a_race_of_each_channel_at_once() {
    let server = MemoryServer::new();
    server.set_connects(Connects::Refused);
    let (parts, _groups) = cluster(&server);
    sleep(120 * SECOND).await;

    let raised = Instant::now();
    parts.signal.raise(Cause::RetryNow);
    sleep(10 * SECOND).await;
    let tries = tries_in(&server, raised, Instant::now());
    let races: Vec<_> = tries.iter().filter(|(_, effort)| *effort == Effort::Race).collect();
    assert_eq!(races, [&(Duration::ZERO, Effort::Race); 3], "{tries:?}");
    let singles: Vec<_> = tries.iter().filter(|(_, effort)| *effort == Effort::Single).collect();
    assert!((1..=2).contains(&singles.len()), "the turns go on one spacing after the races: {tries:?}");
    assert!(singles.iter().all(|(after, _)| *after >= 4 * SECOND), "{tries:?}");
}

/// A second fault in the recovery: the path lets one connect through and fails again. The
/// connect ends the outage, and the races after it start a new one with a first part of its
/// own. The channel that connected stays live, and the others take turns again.
#[tokio::test(start_paused = true)]
async fn a_path_that_fails_again_after_one_connect_starts_a_new_outage_and_the_turns_come_back() {
    let server = MemoryServer::new();
    let rationed = Arc::new(Rationed { server: Arc::clone(&server), left: AtomicUsize::new(0) });
    let (parts, groups) = on(Parts { dial: Arc::clone(&rationed) as _, ..server.parts() });
    sleep(300 * SECOND).await;
    assert!(parts.outage.status().is_some_and(|status| status.turn.is_some()), "the channels take turns");

    rationed.left.store(1, Ordering::SeqCst);
    let mut changes = parts.changed.watch();
    let live = |groups: &[Group]| {
        let first = groups.iter().filter_map(|group| group.status().channels.first().map(|first| first.state.clone()));
        first.filter(|state| *state == ChannelState::Live).count()
    };
    while live(&groups) == 0 {
        changes.next().await;
    }
    let back = Instant::now();
    let tries = tries_in(&server, back, back + MS);
    let [(_, Effort::Single), (_, Effort::Race), (_, Effort::Race)] = tries[..] else {
        panic!("the turn that connected, then a race of each other channel at once: {tries:?}");
    };
    let status = parts.outage.status().expect("the races that failed start a new outage");
    assert_eq!(status.turn, None, "a new outage starts with its first part");

    sleep(120 * SECOND).await;
    assert_eq!(live(&groups), 1, "the channel that connected stays live");
    let first = tries_in(&server, back + MS, back + 60 * SECOND);
    assert!(first.len() > 20 && first.iter().all(|(_, effort)| *effort == Effort::Race), "{first:?}");
    let later = tries_in(&server, back + 60 * SECOND, Instant::now());
    assert!(later.iter().all(|(_, effort)| *effort == Effort::Single), "then the turns again: {later:?}");
    for pair in later.windows(2) {
        let [(one, _), (two, _)] = pair else { continue };
        assert!((4 * SECOND..=6 * SECOND).contains(&(*two - *one)), "one turn each 5 s with jitter: {pair:?}");
    }

    rationed.left.store(usize::MAX, Ordering::SeqCst);
    let returned = Instant::now();
    while !each_live(&groups) {
        changes.next().await;
    }
    assert!(returned.elapsed() <= 6 * SECOND, "the next turn connects: {:?}", returned.elapsed());
    assert_eq!(parts.outage.status(), None);
}
