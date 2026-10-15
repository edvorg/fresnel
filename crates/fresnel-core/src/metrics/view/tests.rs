use std::collections::{BTreeMap, BTreeSet};

use super::*;
use crate::metrics::resources::Amount;
use crate::metrics::series::Cover;
use crate::metrics::world::Known;

const START: u32 = 1_792_058_400;
const PERIOD: Duration = Duration::from_secs(15);

fn at(seconds: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)
}

fn node(name: &str, at: u32, window: Option<u64>, cpu: u64) -> NodeSample {
    let window = window.map(Duration::from_secs);
    NodeSample { name: name.into(), at, window, usage: Amount { cpu, memory: 1 } }
}

fn read<T>(items: Vec<T>) -> Result<Read<T>, CoreError> {
    Ok(Read { items, unread: 0, problem: None })
}

fn poll(pods: Result<Read<PodSample>, CoreError>, nodes: Result<Read<NodeSample>, CoreError>) -> Poll {
    Poll { pods, nodes, service: None }
}

fn world(nodes: &[&str]) -> World {
    let nodes = nodes.iter().map(|name| Arc::from(*name)).collect::<BTreeSet<_>>();
    World {
        nodes: Known { listed: true, items: nodes },
        loads: [(Arc::from("node-a"), NodeLoad::default())].into(),
        ..World::default()
    }
}

fn not_installed() -> CoreError {
    CoreError::answered(404, "the server could not find the requested resource (HTTP 404)")
}

/// Weakness 2 of plan section 6: a match by name without the cluster.
#[test]
fn a_lookup_with_an_other_cluster_finds_nothing() {
    let (ams, fra) = (ClusterId::new("ams"), ClusterId::new("fra"));
    let mut metrics = Metrics::waiting(ams.clone(), PERIOD, at(0));
    let settings = MetricsSettings::default();
    metrics.take(poll(read(vec![]), read(vec![node("node-a", START, None, 7)])), world(&["node-a"]), at(1), &settings);
    let key = SeriesKey::Node("node-a".into());
    assert_eq!(metrics.latest(&ams, &key).map(|latest| latest.sample.usage.cpu), Some(7));
    assert_eq!(metrics.latest(&fra, &key), None);
    assert!(metrics.series(&fra, &key).is_none() && metrics.series(&ams, &key).is_some());
    assert_eq!(
        (metrics.node_load(&ams, "node-a"), metrics.node_load(&fra, "node-a")),
        (Some(NodeLoad::default()), None)
    );
    let pod = ObjectKey::new("shop", "web");
    assert_eq!(metrics.containers(&fra, &pod).count(), 0);
}

#[test]
fn a_value_is_current_while_its_list_works_and_has_the_object_and_stays_old_after() {
    let ams = ClusterId::new("ams");
    let settings = MetricsSettings::default();
    let mut metrics = Metrics::waiting(ams.clone(), PERIOD, at(0));
    let both = vec![node("node-a", START, None, 1), node("node-b", START, None, 2)];
    metrics.take(poll(read(vec![]), read(both)), world(&["node-a", "node-b"]), at(15), &settings);
    let latest = |key: &str| metrics.latest(&ams, &SeriesKey::Node(key.into())).map(|latest| latest.current);
    assert_eq!((latest("node-a"), latest("node-b")), (Some(true), Some(true)));

    // The same sample again is still current; a node that is not in the list is old.
    metrics.take(
        poll(read(vec![]), read(vec![node("node-a", START, None, 1)])),
        world(&["node-a", "node-b"]),
        at(30),
        &settings,
    );
    let latest = |key: &str| metrics.latest(&ams, &SeriesKey::Node(key.into())).map(|latest| latest.current);
    assert_eq!((latest("node-a"), latest("node-b")), (Some(true), Some(false)));
    let cluster = metrics.latest(&ams, &SeriesKey::Cluster).map(|latest| latest.sample.cover);
    assert_eq!(cluster, Some(Cover { sampled: 2, of: 2 }), "the sum of the first answer: the second had no later time");

    // Weakness 4 of plan section 6: a failed list keeps each value, and shows it as old.
    metrics.take(poll(read(vec![]), Err(not_installed())), world(&["node-a", "node-b"]), at(45), &settings);
    assert_eq!((&metrics.nodes.state, metrics.nodes.since), (&PollState::NotInstalled(not_installed()), at(45)));
    let latest = metrics.latest(&ams, &SeriesKey::Node("node-a".into()));
    assert_eq!(latest.map(|latest| (latest.sample.usage.cpu, latest.current)), Some((1, false)));
    assert_eq!(metrics.pods.state, PollState::Ok, "the other list of the cluster keeps its state");
    assert_eq!(metrics.next, Some(at(60)));
}

/// Research 11 section 4.6: a cluster with no `metrics.k8s.io` gets no poll each period.
#[test]
fn a_cluster_with_no_metrics_api_has_its_next_poll_after_the_long_pause() {
    let mut metrics = Metrics::waiting(ClusterId::new("ams"), PERIOD, at(0));
    let settings = MetricsSettings::default();
    metrics.take(poll(Err(not_installed()), read(vec![])), World::default(), at(15), &settings);
    assert_eq!((metrics.pause(), metrics.next), (PERIOD, Some(at(30))), "one list works");
    metrics.take(poll(Err(not_installed()), Err(not_installed())), World::default(), at(30), &settings);
    assert_eq!((metrics.pause(), metrics.next), (ABSENT_PAUSE, Some(at(330))));
}

/// Bad data of a cluster: one sample of a clock that is one day ahead. No later answer has
/// a later time, so the series keeps that sample, and its value shows as old. A sample of a
/// second replica of the server, a little older than the last one, keeps it current.
#[test]
fn a_value_that_no_later_answer_replaces_shows_as_old() {
    let ams = ClusterId::new("ams");
    let settings = MetricsSettings::default();
    let mut metrics = Metrics::waiting(ams.clone(), PERIOD, at(0));
    let mut answer = |at_cluster: u32, now| {
        let nodes = read(vec![node("node-a", at_cluster, None, 1)]);
        metrics.take(poll(read(vec![]), nodes), world(&["node-a"]), at(now), &settings);
        let latest = metrics.latest(&ams, &SeriesKey::Node("node-a".into()));
        latest.map(|latest| (latest.sample.at - START, latest.current))
    };
    assert_eq!(answer(START + 86_400, 15), Some((86_400, true)));
    assert_eq!(answer(START + 15, 30), Some((86_400, false)), "the sample of the last answer is one day older");
    assert_eq!(answer(START + 86_400 - 10, 45), Some((86_400, true)), "a second replica");
}

#[test]
fn a_longer_window_of_the_server_sets_the_period_up_to_one_minute() {
    let ams = ClusterId::new("ams");
    let settings = MetricsSettings::default();
    let mut metrics = Metrics::waiting(ams, PERIOD, at(0));
    let nodes = |windows: [Option<u64>; 3]| {
        let names = ["node-a", "node-b", "node-c"];
        let nodes = names.iter().zip(windows).map(|(name, window)| node(name, START, window, 1)).collect();
        read(nodes)
    };
    let mut period_of = |windows| {
        metrics.take(poll(read(vec![]), nodes(windows)), World::default(), at(0), &settings);
        metrics.period
    };
    assert_eq!(period_of([Some(10), Some(10), Some(10)]), PERIOD, "a shorter window keeps the setting");
    assert_eq!(period_of([Some(30), Some(20), Some(90)]), Duration::from_secs(30), "the median window");
    assert_eq!(period_of([Some(300), Some(300), None]), Duration::from_secs(60), "one minute at most");
    assert_eq!(period_of([None, None, None]), PERIOD);
}

#[test]
fn a_new_state_of_both_lists_keeps_each_value_and_has_no_next_poll() {
    let ams = ClusterId::new("ams");
    let mut metrics = Metrics::waiting(ams.clone(), PERIOD, at(0));
    let settings = MetricsSettings::default();
    metrics.take(poll(read(vec![]), read(vec![node("node-a", START, None, 1)])), world(&["node-a"]), at(15), &settings);
    let stopped = metrics.in_state(&PollState::Stopped, at(20));
    assert_eq!(
        (&stopped.pods.state, &stopped.nodes.state, stopped.next),
        (&PollState::Stopped, &PollState::Stopped, None)
    );
    assert_eq!((stopped.nodes.since, stopped.series_count(), stopped.bytes()), (at(20), 2, metrics.bytes()));
    assert_eq!(stopped.dropped(), 0);
}

/// The metrics of a test of a reader: each sample goes to the series of its key, a node to
/// the NodeMetrics and each other object to the PodMetrics. A series that the next answer
/// has no sample of is old.
#[test]
fn the_samples_of_a_test_give_each_series_its_value_and_count_one_answer_of_each_list() {
    let ams = ClusterId::new("ams");
    let settings = MetricsSettings::default();
    let mut metrics = Metrics::waiting(ams.clone(), PERIOD, at(0));
    let sample = |at, cpu| Sample { at, cover: Cover::ONE, usage: Amount { cpu, memory: 2 } };
    let (pod, node) = (SeriesKey::Pod(ObjectKey::new("shop", "web")), SeriesKey::Node("node-a".into()));
    let load = NodeLoad { requests: Amount { cpu: 5, memory: 6 }, pods: 1 };
    let loads = [(Arc::from("node-a"), load)].into();
    metrics.take_samples(
        &[(pod.clone(), sample(START, 3)), (node.clone(), sample(START, 4))],
        loads,
        at(15),
        &settings,
    );
    let value = |key: &SeriesKey| metrics.latest(&ams, key).map(|latest| (latest.sample.usage.cpu, latest.current));
    assert_eq!((value(&pod), value(&node)), (Some((3, true)), Some((4, true))));
    assert_eq!((metrics.pods.items, metrics.nodes.items, metrics.node_load(&ams, "node-a")), (1, 1, Some(load)));
    assert_eq!(
        (&metrics.pods.state, metrics.pods.answered, metrics.next),
        (&PollState::Ok, Some(at(15)), Some(at(30)))
    );

    metrics.take_samples(&[(node.clone(), sample(START + 15, 7))], BTreeMap::new(), at(30), &settings);
    let value = |key: &SeriesKey| metrics.latest(&ams, key).map(|latest| (latest.sample.usage.cpu, latest.current));
    assert_eq!((value(&pod), value(&node)), (Some((3, false)), Some((7, true))));
    assert_eq!(metrics.series(&ams, &node).map(|series| series.samples().count()), Some(2));
}
