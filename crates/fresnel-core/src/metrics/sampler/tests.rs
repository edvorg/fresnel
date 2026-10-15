use std::sync::Mutex;
use std::sync::mpsc;
use std::time::{Duration, SystemTime};

use fresnel_net::ErrorClass;
use fresnel_net::task::TaskReport;
use serde_json::json;
use tokio::runtime::Handle;

use super::*;
use crate::metrics::history::SeriesKey;
use crate::metrics::world;
use crate::published::Disposal;
use crate::store::ObjectKey;

const START: u32 = 1_792_058_400;

/// The answer of the server to the list of one path at one poll, counted from 0.
type Answer = fn(&'static str, u32) -> Result<RawList, CoreError>;

/// The lists of a test: each call is one poll of its path, with its time.
#[derive(Clone)]
struct Script {
    answer: Answer,
    calls: Arc<Mutex<Vec<(&'static str, Duration)>>>,
    start: Instant,
}

impl Lists for Script {
    async fn list(&self, path: &'static str) -> Result<RawList, CoreError> {
        let poll = {
            let mut calls = self.calls.lock().expect("a lock");
            calls.push((path, self.start.elapsed()));
            calls.iter().filter(|(called, _)| *called == path).count() - 1
        };
        (self.answer)(path, u32::try_from(poll).expect("a count"))
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
        let parts =
            SamplerParts { settings: MetricsSettings::default(), bin: disposal.bin(), notice: Notice::new(), clock };
        let inputs = world::tests::scene(&disposal.bin());
        let sampler = Sampler::with_lists(&tasks, ClusterId::new("ams"), script.clone(), inputs, parts);
        let reader = sampler.reader();
        Self { reports, script, sampler, reader, _disposal: disposal }
    }

    /// The metrics after the next publish.
    async fn next(&mut self) -> Metrics {
        self.reader.changed().await;
        Metrics::clone(&self.reader.read())
    }

    fn calls(&self) -> Vec<(&'static str, Duration)> {
        self.script.calls.lock().expect("a lock").clone()
    }
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
    let words =
        ("metrics-server not available: service unavailable (HTTP 503)", "no answer in time: no answer in 10 s");
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

    let seconds = |calls: Vec<(&'static str, Duration)>| -> Vec<(&'static str, u64)> {
        calls.into_iter().map(|(path, at)| (path, at.as_secs())).collect()
    };
    let polls: Vec<_> = [0, 15, 30, 45].into_iter().flat_map(|at| [(wire::PODS, at), (wire::NODES, at)]).collect();
    assert_eq!(seconds(bed.calls()), polls, "no retry between two periods");
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
    reader.changed().await;
    let stopped = Metrics::clone(&reader.read());
    assert_eq!(states(&stopped), ("stopped".to_owned(), "stopped".to_owned()));
    assert_eq!(stopped.series_count(), first.series_count());
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert_eq!(script.calls.lock().expect("a lock").len(), 2, "no poll after the stop");
}
