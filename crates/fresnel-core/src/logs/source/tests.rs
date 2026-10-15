//! The source on a script: the test answers each open, sends the data of each stream and
//! changes the pod in its store. The time is paused, so each pause is exact.

use std::time::{Duration, SystemTime};

use fresnel_net::task::{Backoff, TaskHandle, TaskReport, Tasks};
use fresnel_net::{ErrorClass, Signal, WallClock};
use serde_json::json;
use tokio::runtime::Handle;
use tokio::sync::mpsc;

use super::*;
use crate::logs::ingest::NANOS;
use crate::logs::testing::{Call, Script, Store, data, pod, running};
use crate::published::Disposal;

struct Bed {
    _source: TaskHandle,
    calls: mpsc::UnboundedReceiver<Call>,
    reports: mpsc::Receiver<Report>,
    pods: Store<Pod>,
    signal: Signal,
    _disposal: Disposal,
}

const RETRY: Backoff = Backoff {
    first: Duration::from_millis(100),
    max: Duration::from_secs(3),
    jitter_percent: 0,
    reset_after: Duration::ZERO,
};

impl Bed {
    fn start(pods: Vec<Pod>, previous: bool) -> Self {
        let tasks = Tasks::new(Handle::current(), |report: TaskReport| panic!("a task ended: {report}"));
        let disposal = Disposal::start(&tasks);
        let pods = Store::new(&disposal, pods);
        let (script, calls) = Script::new();
        let (sender, reports) = mpsc::channel(16);
        let signal = Signal::new();
        let stream =
            Stream { index: 2, pod: ObjectKey::new("shop", "web-1"), uid: "u1".into(), container: "app".into() };
        let parts = SourceParts {
            stream,
            previous,
            first: Origin::Options { tail: Some(1000), since: None },
            opener: script,
            pods: pods.reader(),
            signal: signal.clone(),
            reports: sender,
            retry: RETRY,
            clock: WallClock::new(|| SystemTime::UNIX_EPOCH),
        };
        let source = start(&tasks, &ClusterId::new("ams"), RETRY, 1024, parts);
        Self { _source: source, calls, reports, pods, signal, _disposal: disposal }
    }

    async fn call(&mut self) -> Call {
        self.calls.recv().await.expect("an open")
    }

    /// The next report in words: a state, or the lines with their times in seconds.
    async fn next(&mut self) -> String {
        match self.reports.recv().await.expect("a report") {
            Report::State { source, state } => format!("{source}: {state}"),
            Report::Lines { source, lines, repeated } => {
                let line = |line: &LogLine| {
                    let time = line.time as f64 / NANOS as f64;
                    format!("{time}{} {}", if line.kind == LineKind::Marker { " marker" } else { "" }, line.text)
                };
                format!("{source}: {} (repeated {repeated})", lines.iter().map(line).collect::<Vec<_>>().join(", "))
            }
        }
    }

    async fn reports(&mut self, count: usize) -> Vec<String> {
        let mut reports = Vec::new();
        for _ in 0..count {
            reports.push(self.next().await);
        }
        reports
    }
}

fn since(origin: &Origin) -> String {
    match origin {
        Origin::Options { tail, since } => format!("options {tail:?} {since:?}"),
        Origin::Time(time) => format!("from {time}"),
        Origin::Whole => "whole".to_owned(),
    }
}

#[tokio::test(start_paused = true)]
async fn a_break_gives_a_new_stream_from_the_second_of_the_latest_line_with_a_marker_and_no_line_two_times() {
    let mut bed = Bed::start(vec![running("containerd://a1")], false);
    let first = bed.call().await;
    assert_eq!((first.pod.name.as_ref(), first.container.as_str(), first.previous), ("web-1", "app", false));
    assert_eq!(since(&first.origin), "options Some(1000) None");
    let stream = first.open();
    stream.send(data(&[(10.1, "one"), (11.2, "two"), (11.5, "three")])).expect("open");
    assert_eq!(
        bed.reports(3).await,
        ["2: connecting", "2: streaming", "2: 10.1 one, 11.2 two, 11.5 three (repeated 0)"]
    );

    let started = tokio::time::Instant::now();
    stream.send(Err(CoreError::new(ErrorClass::Network, "the connection broke"))).expect("open");
    assert_eq!(bed.next().await, "2: reconnecting after: the connection broke");
    let second = bed.call().await;
    assert_eq!((since(&second.origin), started.elapsed()), ("from 1970-01-01T00:00:11Z".to_owned(), RETRY.first));
    let stream = second.open();
    stream.send(data(&[(11.2, "two"), (11.5, "three"), (11.7, "four")])).expect("open");
    let marker = "2: 11.5 marker the stream started again after: the connection broke (repeated 0)";
    assert_eq!(bed.reports(4).await, ["2: connecting", "2: streaming", marker, "2: 11.7 four (repeated 2)"]);
}

#[tokio::test(start_paused = true)]
async fn a_container_that_ends_and_starts_again_gives_a_wait_and_a_marker_before_its_new_lines() {
    let mut bed = Bed::start(vec![running("containerd://a1")], false);
    let stream = bed.call().await.open();
    stream.send(data(&[(10.0, "first run")])).expect("open");
    assert_eq!(bed.reports(3).await[2], "2: 10 first run (repeated 0)");

    // The stream ends before the store has the end of the container.
    drop(stream);
    assert_eq!(bed.next().await, "2: reconnecting after: the stream ended while the container runs");
    bed.pods.set(vec![pod("web-1", "u1", json!({"waiting": {"reason": "CrashLoopBackOff"}}), |pod| {
        pod["status"]["containerStatuses"][0]["lastState"] =
            json!({"terminated": {"exitCode": 1, "reason": "Error", "containerID": "containerd://a1"}});
    })]);
    assert_eq!(bed.next().await, "2: waiting: the container ended (Error, exit code 1) and starts again");

    bed.pods.set(vec![running("containerd://a2")]);
    let call = bed.call().await;
    assert_eq!(since(&call.origin), "from 1970-01-01T00:00:10Z");
    let stream = call.open();
    stream.send(data(&[(12.0, "second run")])).expect("open");
    let reports = bed.reports(4).await;
    assert_eq!(reports[2], "2: 10 marker the container app started again (repeated 0)");
    assert_eq!(reports[3], "2: 12 second run (repeated 0)");
}

#[tokio::test(start_paused = true)]
async fn a_container_that_did_not_start_waits_with_its_reason_and_sends_no_request() {
    let waiting = json!({"waiting": {"reason": "ImagePullBackOff", "message": "Back-off pulling image \"nope\""}});
    let mut bed = Bed::start(vec![pod("web-1", "u1", waiting, |_| {})], false);
    let text = "2: waiting: the container did not start: ImagePullBackOff: Back-off pulling image \"nope\"";
    assert_eq!(bed.next().await, text);
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert!(bed.calls.try_recv().is_err(), "no request for a container with no log");

    bed.pods.set(vec![running("containerd://a1")]);
    let call = bed.call().await;
    assert_eq!(since(&call.origin), "options Some(1000) None", "the first stream has the options");
    // The kubelet can refuse while the store has the start already.
    call.refuse(CoreError::answered(400, "container \"app\" in pod \"web-1\" is waiting to start: ContainerCreating"));
    assert_eq!(
        bed.reports(2).await[1],
        "2: waiting: container \"app\" in pod \"web-1\" is waiting to start: ContainerCreating"
    );
    let _open = bed.call().await.open();
    assert_eq!(bed.reports(2).await, ["2: connecting", "2: streaming"]);
}

#[tokio::test(start_paused = true)]
async fn a_container_that_completed_ends_after_its_full_log_and_a_gone_pod_ends_its_source() {
    let completed = json!({"terminated": {"exitCode": 0, "reason": "Completed", "containerID": "containerd://a1"}});
    let done = pod("web-1", "u1", completed, |pod| {
        pod["spec"]["restartPolicy"] = json!("Never");
        pod["status"]["phase"] = json!("Succeeded");
    });
    let mut bed = Bed::start(vec![done], false);
    let stream = bed.call().await.open();
    stream.send(data(&[(5.0, "done")])).expect("open");
    drop(stream);
    assert_eq!(bed.reports(4).await[3], "2: ended: the container ended: Completed, exit code 0");

    let mut bed = Bed::start(vec![running("containerd://a1")], false);
    bed.call().await.refuse(CoreError::answered(404, "pods \"web-1\" not found"));
    assert_eq!(bed.reports(2).await[1], "2: reconnecting after: pods \"web-1\" not found");
    bed.pods.set(
        vec![running("containerd://a1")]
            .into_iter()
            .map(|mut pod| {
                pod.metadata.uid = Some("u2".to_owned());
                pod
            })
            .collect(),
    );
    assert_eq!(bed.next().await, "2: ended: the pod is gone", "a new pod of the name is an other pod");
}

#[tokio::test(start_paused = true)]
async fn the_earlier_instance_is_one_stream_with_no_follow_and_a_refusal_of_the_rules_fails() {
    let crashed = pod("web-1", "u1", json!({"running": {}}), |pod| {
        pod["status"]["containerStatuses"][0]["lastState"] =
            json!({"terminated": {"exitCode": 2, "reason": "Error", "containerID": "containerd://a0"}});
    });
    let mut bed = Bed::start(vec![crashed], true);
    let call = bed.call().await;
    assert_eq!((call.previous, since(&call.origin)), (true, "options Some(1000) None".to_owned()));
    let stream = call.open();
    stream.send(data(&[(3.0, "the crash")])).expect("open");
    drop(stream);
    let reports = bed.reports(4).await;
    assert_eq!(
        reports[2..],
        ["2: 3 the crash (repeated 0)", "2: ended: the view has the full log of the earlier instance"]
    );

    let mut bed = Bed::start(vec![running("containerd://a1")], true);
    assert_eq!(bed.next().await, "2: ended: the container has no earlier instance");

    let mut bed = Bed::start(vec![running("containerd://a1")], false);
    bed.call().await.refuse(CoreError::answered(403, "pods \"web-1\" is forbidden"));
    assert_eq!(bed.reports(2).await[1], "2: failed: pods \"web-1\" is forbidden");
    bed.signal.raise(fresnel_net::Cause::RetryNow);
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert!(bed.calls.try_recv().is_err(), "a refusal of the rules sends no new request");
}
