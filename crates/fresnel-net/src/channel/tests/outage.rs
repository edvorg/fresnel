//! The channels of one cluster in a long outage: one attempt for each turn of the cluster.

use super::*;
use crate::connect::Effort;

/// The three groups of a cluster, on one server and one outage.
fn cluster(server: &Arc<MemoryServer>) -> (Parts, Vec<Group>) {
    let parts = server.parts();
    let purposes = [Purpose::Watches, Purpose::Requests, Purpose::Logs];
    let groups = purposes.map(|purpose| Group::start(parts.clone(), purpose, settings())).into();
    (parts, groups)
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
