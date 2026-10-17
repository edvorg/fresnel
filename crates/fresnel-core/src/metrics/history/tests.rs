use std::collections::BTreeSet;

use super::*;
use crate::metrics::resources::Amount;
use crate::metrics::series::CHUNK_BYTES;
use crate::metrics::series::Cover;

const TIERS: Tiers = Tiers { fine: 3_600, step: 60, coarse: 86_400 };
const START: u32 = 1_792_058_400;
const HOUR: Duration = Duration::from_secs(3_600);

fn sample(at: u32) -> Sample {
    Sample { at, cover: Cover::ONE, usage: Amount { cpu: 1, memory: 1 } }
}

fn pod(name: &str) -> ObjectKey {
    ObjectKey::new("shop", name)
}

fn container(pod_name: &str, name: &str) -> SeriesKey {
    SeriesKey::Container { pod: pod(pod_name), name: name.into() }
}

fn at(seconds: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)
}

#[test]
fn the_containers_of_a_pod_come_in_the_sequence_of_their_names_and_no_other_pod_comes() {
    let mut history = History::default();
    for key in [
        container("web-2", "app"),
        container("web-1", "proxy"),
        container("web-1", "app"),
        SeriesKey::Pod(pod("web-1")),
    ] {
        history.record(key, sample(START), 1, &TIERS);
    }
    let web = pod("web-1");
    let names: Vec<&str> = history.containers(&web).map(|(name, _)| name).collect();
    assert_eq!(names, ["app", "proxy"]);
    assert_eq!(history.containers(&pod("web-3")).count(), 0);
    assert_eq!((history.len(), history.samples), (4, 4));
}

#[test]
fn a_sample_again_marks_its_series_as_seen_and_adds_nothing() {
    let mut history = History::default();
    history.record(SeriesKey::Cluster, sample(START), 1, &TIERS);
    history.record(SeriesKey::Cluster, sample(START), 2, &TIERS);
    let series = history.get(&SeriesKey::Cluster).expect("the series");
    assert_eq!((series.seen, series.len(), history.samples), (2, 1, 1));
}

/// Plan section 12.4: the drop of a deleted pod after one hour.
#[test]
fn the_series_of_an_object_that_left_its_store_go_one_hour_later() {
    let mut history = History::default();
    for key in [SeriesKey::Pod(pod("web-1")), container("web-1", "app"), SeriesKey::Pod(pod("web-2"))] {
        history.record(key, sample(START), 1, &TIERS);
    }
    let pods = |names: &[&str]| -> BTreeSet<ObjectKey> { names.iter().map(|name| pod(name)).collect() };
    let exists = |store: Option<BTreeSet<ObjectKey>>| {
        move |key: &SeriesKey| match key {
            SeriesKey::Pod(pod) | SeriesKey::Container { pod, .. } => store.as_ref().map(|store| store.contains(pod)),
            _ => Some(true),
        }
    };
    history.sweep(exists(Some(pods(&["web-1", "web-2"]))), at(0), HOUR);
    history.sweep(exists(Some(pods(&["web-2"]))), at(60), HOUR);
    // A store with no complete list changes no mark.
    history.sweep(exists(None), at(1_800), HOUR);
    history.sweep(exists(Some(pods(&["web-2"]))), at(3_659), HOUR);
    assert_eq!(history.len(), 3, "59 minutes and 59 seconds after the delete");
    history.sweep(exists(Some(pods(&["web-2"]))), at(3_660), HOUR);
    let left: Vec<_> = history.series.keys().cloned().collect();
    assert_eq!((left, history.samples), (vec![SeriesKey::Pod(pod("web-2"))], 1));

    // A pod that comes back loses its mark: a StatefulSet makes a pod of the same name.
    history.sweep(exists(Some(pods(&[]))), at(4_000), HOUR);
    history.sweep(exists(Some(pods(&["web-2"]))), at(5_000), HOUR);
    history.sweep(exists(Some(pods(&["web-2"]))), at(9_000), HOUR);
    assert_eq!(history.len(), 1);
    // A clock that goes back keeps the series.
    history.sweep(exists(Some(pods(&[]))), at(10_000), HOUR);
    history.sweep(exists(Some(pods(&[]))), at(5), HOUR);
    assert_eq!(history.len(), 1);
}

/// Bad data of a server: a sample at the last second of the time of a sample. Each step of
/// the limit drops a sample, so the limit ends.
#[test]
fn the_memory_limit_ends_with_a_sample_at_the_last_second() {
    let mut history = History::default();
    history.record(SeriesKey::Cluster, sample(u32::MAX - 15), 1, &TIERS);
    history.record(SeriesKey::Cluster, sample(u32::MAX), 2, &TIERS);
    history.limit(0);
    assert_eq!((history.samples, history.dropped), (0, 2));
}

/// Two hours of samples each 15 s of one series of each rank.
fn full_history() -> History {
    let mut history = History::default();
    let keys = [
        container("web-1", "app"),
        SeriesKey::Pod(pod("web-1")),
        SeriesKey::Workload(Kind::Deployment, pod("web")),
        SeriesKey::Namespace("shop".into()),
        SeriesKey::Node("node-a".into()),
        SeriesKey::Cluster,
    ];
    for step in 0..480u32 {
        for key in &keys {
            history.record(key.clone(), sample(START + step * 15), u64::from(step), &TIERS);
        }
    }
    history
}

fn kept(history: &History, key: &SeriesKey, tier: Tier) -> Option<u32> {
    history.get(key).and_then(|series| series.oldest(tier)).map(|oldest| oldest - START)
}

/// Research 11 section 4.3: when the limit is reached, the coarse tier of the containers
/// goes first. A tier holds its samples in chunks of 64, so a drop frees memory when a chunk
/// empties.
#[test]
fn the_memory_limit_drops_the_oldest_coarse_samples_of_the_containers_first() {
    let mut history = full_history();
    let full = history.bytes();
    assert_eq!(history.samples, 6 * (240 + 119));
    assert_eq!(full, 6 * (7 + 5) * CHUNK_BYTES + 6 * SERIES_BYTES, "240 fine and 119 coarse samples");
    history.limit(full);
    assert_eq!((history.bytes(), history.dropped), (full, 0), "a history in its limit");

    // The coarse tier of the container keeps 64 samples, one chunk in place of five.
    history.limit(full - CHUNK_BYTES);
    assert_eq!(history.bytes(), full - 4 * CHUNK_BYTES);
    let app = container("web-1", "app");
    assert_eq!(kept(&history, &app, Tier::Coarse), Some(55 * 60 + 45), "55 minutes of the container went");
    assert_eq!(kept(&history, &SeriesKey::Pod(pod("web-1")), Tier::Coarse), Some(45));
    assert_eq!(history.dropped, 55);

    // A limit below each coarse tier: the coarse tiers are empty, the fine tier of the
    // containers loses its oldest minutes until it holds a chunk less, and the cluster keeps
    // its fine tier.
    let fine = 6 * 7 * CHUNK_BYTES + 6 * SERIES_BYTES;
    history.limit(fine - 1);
    assert_eq!(history.bytes(), fine - CHUNK_BYTES);
    for key in history.series.keys() {
        assert_eq!(kept(&history, key, Tier::Coarse), None, "{key:?}");
    }
    assert_eq!(kept(&history, &app, Tier::Fine), Some(3_600 + 12 * 60), "12 minutes of the fine tier went");
    assert_eq!(kept(&history, &SeriesKey::Cluster, Tier::Fine), Some(3_600));
    assert_eq!(history.dropped, 6 * 119 + 48);

    // No room for any sample: each series keeps its key and its last value.
    history.limit(0);
    assert_eq!((history.samples, history.len(), history.bytes()), (0, 6, 6 * SERIES_BYTES));
    assert_eq!(history.get(&SeriesKey::Cluster).and_then(Series::latest), Some(sample(START + 479 * 15)));
}

/// A tier of 3 samples holds a chunk of 64. Where pods come and go, most series are short:
/// with 24 bytes for each sample, the limit counted a part of the heap of the history only,
/// and a fast soak measured more than twice its limit.
#[test]
fn the_memory_limit_counts_a_chunk_for_a_short_tier() {
    let mut history = History::default();
    for number in 0..100 {
        for step in 0..3 {
            let key = SeriesKey::Pod(pod(&format!("web-{number}")));
            history.record(key, sample(START + number * 60 + step * 15), 1, &TIERS);
        }
    }
    assert_eq!(history.bytes(), 100 * (CHUNK_BYTES + SERIES_BYTES));
    let limit = 100 * SERIES_BYTES + 50 * CHUNK_BYTES;
    history.limit(limit);
    assert_eq!((history.bytes(), history.dropped, history.samples), (limit, 150, 150), "the 50 oldest minutes went");
    assert_eq!(kept(&history, &SeriesKey::Pod(pod("web-50")), Tier::Fine), Some(50 * 60));
}

/// The memory account measures a copy of the history.
#[test]
fn a_rebuilt_history_has_each_series_with_each_sample() {
    let history = full_history();
    let copy = history.rebuilt();
    assert_eq!((copy.len(), copy.samples, copy.bytes()), (history.len(), history.samples, history.bytes()));
    for (key, series) in &history.series {
        let rebuilt = copy.get(key).expect("each series");
        assert!(series.samples().eq(rebuilt.samples()), "{key:?}");
    }
}
