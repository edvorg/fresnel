use std::sync::Mutex;
use std::sync::mpsc;
use std::time::{Duration, SystemTime};

use fresnel_net::task::TaskReport;
use fresnel_net::{Cause, ErrorClass};
use serde_json::json;
use tokio::runtime::Handle;

use super::*;
use crate::metrics::history::SeriesKey;
use crate::metrics::state::ABSENT_PAUSE;
use crate::metrics::world;
use crate::published::Disposal;
use crate::store::ObjectKey;

const START: u32 = 1_792_058_400;

/// A limit for each wait of a test: with paused time, a sampler that never publishes would
/// make a wait with no limit run with no end.
const WAIT_LIMIT: Duration = Duration::from_secs(3_600);
/// The path of the APIService in the calls of a script.
const SERVICE: &str = "service";

/// The answer of the server to the list of one path at one poll, counted from 0.
type Answer = fn(&'static str, u32) -> Result<RawList, CoreError>;

/// The lists of a test: each call is one poll of its path, with its time.
#[derive(Clone)]
struct Script {
    answer: Answer,
    calls: Arc<Mutex<Vec<(&'static str, Duration)>>>,
    start: Instant,
}

impl Script {
    /// Counts one call of a path and gives its number, from 0.
    fn call(&self, path: &'static str) -> u32 {
        let mut calls = self.calls.lock().expect("a lock");
        calls.push((path, self.start.elapsed()));
        u32::try_from(calls.iter().filter(|(called, _)| *called == path).count() - 1).expect("a count")
    }
}

impl Lists for Script {
    async fn list(&self, path: &'static str) -> Result<RawList, CoreError> {
        (self.answer)(path, self.call(path))
    }

    /// The APIService of a backend that does not answer.
    async fn service(&self) -> Result<RawService, CoreError> {
        self.call(SERVICE);
        let condition = json!({ "type": "Available", "status": "False", "reason": "FailedDiscoveryCheck",
            "message": "failing or missing response from https://10.43.0.9:10250" });
        Ok(serde_json::from_value(json!({ "status": { "conditions": [condition] } })).expect("an APIService"))
    }
}

/// One pod with one container and one node, with the time of the poll.
fn items(path: &str, poll: u32) -> RawList {
    let timestamp = jiff_time(START + poll * 15);
    let item = if path == wire::PODS {
        let container = json!({ "name": "app", "usage": { "cpu": "10m", "memory": "1Mi" } });
        json!({ "metadata": { "name": "web", "namespace": "shop" }, "timestamp": timestamp, "containers": [container] })
    } else {
        json!({ "metadata": { "name": "node-a" }, "timestamp": timestamp, "usage": { "cpu": "1", "memory": "1Gi" } })
    };
    serde_json::from_value(json!({ "items": [item] })).expect("a list")
}

fn jiff_time(seconds: u32) -> String {
    k8s_openapi::jiff::Timestamp::from_second(seconds.into()).expect("a time").to_string()
}

struct Bed {
    reports: mpsc::Receiver<TaskReport>,
    script: Script,
    signal: Signal,
    sampler: Sampler,
    reader: Reader<Metrics>,
    _disposal: Disposal,
}

impl Bed {
    fn start(answer: Answer) -> Self {
        let (sender, reports) = mpsc::channel();
        let tasks = Tasks::new(Handle::current(), move |report: TaskReport| sender.send(report).expect("a reader"));
        let disposal = Disposal::start(&tasks);
        let start = Instant::now();
        let clock = WallClock::new(move || SystemTime::UNIX_EPOCH + start.elapsed());
        let script = Script { answer, calls: Arc::default(), start };
        let (settings, signal) = (MetricsSettings::default(), Signal::new());
        let parts =
            SamplerParts { settings, bin: disposal.bin(), notice: Notice::new(), clock, signal: signal.clone() };
        let inputs = world::tests::scene(&disposal.bin());
        let sampler = Sampler::with_lists(&tasks, ClusterId::new("ams"), script.clone(), inputs, parts);
        let reader = sampler.reader();
        Self { reports, script, signal, sampler, reader, _disposal: disposal }
    }

    /// The metrics after the next publish.
    async fn next(&mut self) -> Metrics {
        until_changed(&mut self.reader).await;
        Metrics::clone(&self.reader.read())
    }

    fn calls(&self) -> Vec<(&'static str, Duration)> {
        self.script.calls.lock().expect("a lock").clone()
    }
}

async fn until_changed(reader: &mut Reader<Metrics>) {
    tokio::time::timeout(WAIT_LIMIT, reader.changed()).await.expect("a publish of the sampler");
}

fn seconds(calls: Vec<(&'static str, Duration)>) -> Vec<(&'static str, u64)> {
    calls.into_iter().map(|(path, at)| (path, at.as_secs())).collect()
}

fn states(metrics: &Metrics) -> (String, String) {
    (metrics.pods.state.to_string(), metrics.nodes.state.to_string())
}

#[tokio::test(start_paused = true)]
async fn the_sampler_polls_both_lists_at_once_each_period_and_each_list_has_its_state() {
    let mut bed = Bed::start(|path, poll| match (path, poll) {
        (wire::PODS, 1) => Err(CoreError::answered(503, "service unavailable (HTTP 503)")),
        (wire::NODES, 1) => Err(CoreError::new(ErrorClass::Timeout, "no answer in 10 s")),
        (wire::PODS, 2) => Err(CoreError::answered(404, "not found (HTTP 404)")),
        (wire::NODES, 2) => Err(CoreError::answered(403, "nodes.metrics.k8s.io is forbidden (HTTP 403)")),
        _ => Ok(items(path, poll)),
    });
    let ams = ClusterId::new("ams");
    let pod = SeriesKey::Pod(ObjectKey::new("shop", "web"));
    let first = bed.next().await;
    assert_eq!(states(&first), ("ok".to_owned(), "ok".to_owned()));
    assert_eq!(first.latest(&ams, &pod).map(|latest| (latest.sample.at, latest.current)), Some((START, true)));
    assert_eq!(first.next, Some(SystemTime::UNIX_EPOCH + Duration::from_secs(15)));

    let second = bed.next().await;
    let words = (
        "metrics-server not available: service unavailable (HTTP 503); the APIService v1beta1.metrics.k8s.io has \
         Available=False (FailedDiscoveryCheck): failing or missing response from https://10.43.0.9:10250",
        "no answer in time: no answer in 10 s",
    );
    assert_eq!(states(&second), (words.0.to_owned(), words.1.to_owned()));
    assert_eq!(second.latest(&ams, &pod).map(|latest| latest.current), Some(false), "an old value stays");
    let third = bed.next().await;
    let words =
        ("metrics-server not found: not found (HTTP 404)", "forbidden: nodes.metrics.k8s.io is forbidden (HTTP 403)");
    assert_eq!(states(&third), (words.0.to_owned(), words.1.to_owned()));
    let fourth = bed.next().await;
    assert_eq!(states(&fourth), ("ok".to_owned(), "ok".to_owned()));
    let series = fourth.series(&ams, &pod).expect("the series of the pod");
    assert_eq!(series.samples().map(|sample| sample.at - START).collect::<Vec<_>>(), [0, 45]);
    assert_eq!(fourth.series_count(), 5, "the container, the pod, the namespace, the node and the cluster");

    let mut polls: Vec<_> = [0, 15, 30, 45].into_iter().flat_map(|at| [(wire::PODS, at), (wire::NODES, at)]).collect();
    polls.insert(4, (SERVICE, 15));
    assert_eq!(seconds(bed.calls()), polls, "no retry between two periods, and one read of the APIService");
    assert!(bed.reports.try_recv().is_err(), "no task ended");
}

#[tokio::test(start_paused = true)]
async fn a_panic_shows_as_an_internal_error_and_the_next_run_keeps_the_history() {
    let mut bed = Bed::start(|path, poll| match (path, poll) {
        (wire::PODS, 1) => panic!("index 4 of 3"),
        _ => Ok(items(path, poll)),
    });
    let ams = ClusterId::new("ams");
    let first = bed.next().await;
    let failed = bed.next().await;
    assert_eq!(failed.pods.state, PollState::InternalError("the task panicked: index 4 of 3".into()));
    assert_eq!((failed.series_count(), failed.next), (first.series_count(), None));

    // The new run polls again after the pause of the restart, with the same history.
    let again = bed.next().await;
    assert_eq!(states(&again), ("ok".to_owned(), "ok".to_owned()));
    let pod = again.series(&ams, &SeriesKey::Pod(ObjectKey::new("shop", "web"))).expect("the pod");
    assert_eq!(pod.samples().map(|sample| sample.at - START).collect::<Vec<_>>(), [0, 30]);
    let report = bed.reports.try_recv().expect("the report of the run that ended");
    assert!(report.to_string().contains("index 4 of 3"), "{report}");
}

#[tokio::test(start_paused = true)]
async fn the_drop_of_the_sampler_stops_its_polls_and_shows_the_state_stopped() {
    let mut bed = Bed::start(|path, poll| Ok(items(path, poll)));
    let first = bed.next().await;
    let Bed { sampler, mut reader, script, .. } = bed;
    drop(sampler);
    until_changed(&mut reader).await;
    let stopped = Metrics::clone(&reader.read());
    assert_eq!(states(&stopped), ("stopped".to_owned(), "stopped".to_owned()));
    assert_eq!(stopped.series_count(), first.series_count());
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert_eq!(script.calls.lock().expect("a lock").len(), 2, "no poll after the stop");
}

/// Research 11 section 4.6: a cluster with no metrics API is asked again each 5 minutes, and
/// at once after "retry now", a wake or a network change.
#[tokio::test(start_paused = true)]
async fn a_cluster_with_no_metrics_api_is_asked_again_after_the_long_pause_or_a_raise_of_its_signal() {
    let mut bed = Bed::start(|path, poll| match poll {
        0..=1 => Err(CoreError::answered(404, "the server could not find the requested resource (HTTP 404)")),
        _ => Ok(items(path, poll)),
    });
    let first = bed.next().await;
    assert!(first.pods.state.is_absent() && first.nodes.state.is_absent(), "{:?}", states(&first));
    assert_eq!(first.next, Some(SystemTime::UNIX_EPOCH + ABSENT_PAUSE));
    bed.next().await;
    // The user asks for a retry 10 s after the second poll.
    tokio::time::sleep(Duration::from_secs(10)).await;
    bed.signal.raise(Cause::RetryNow);
    let third = bed.next().await;
    assert_eq!(third.next, Some(SystemTime::UNIX_EPOCH + Duration::from_secs(325)));
    let fourth = bed.next().await;
    assert_eq!(states(&fourth), ("ok".to_owned(), "ok".to_owned()));

    let polls: Vec<_> = [0, 300, 310, 325].into_iter().flat_map(|at| [(wire::PODS, at), (wire::NODES, at)]).collect();
    assert_eq!(seconds(bed.calls()), polls, "the period again after an answer, and no read of the APIService");
}
