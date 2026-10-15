//! A view on a script of streams, with the stores and the owner index of a test. The time
//! is paused, so the hold of the merge is exact.

use std::time::{Duration, SystemTime};

use bytes::Bytes;
use fresnel_net::task::{TaskReport, Tasks};
use k8s_openapi::api::apps::v1::ReplicaSet;
use k8s_openapi::api::batch::v1::Job;
use k8s_openapi::api::discovery::v1::EndpointSlice;
use serde_json::json;
use tokio::runtime::Handle;
use tokio::sync::mpsc;
use tokio::time::timeout;

use super::*;
use crate::cluster::ClusterId;
use crate::error::CoreError;
use crate::index::TestOwners;
use crate::kinds::Kind;
use crate::logs::request::LogOptions;
use crate::logs::testing::{Call, Script, Store, data, pod};
use crate::logs::view::SourceState;
use crate::published::{Changes, Disposal};
use crate::store::ObjectKey;

struct Bed {
    session: LogSession,
    calls: mpsc::UnboundedReceiver<Call>,
    changes: Changes,
    pods: Store<Pod>,
    _owners: TestOwners,
    _others: (Store<ReplicaSet>, Store<Job>, Store<EndpointSlice>),
    _disposal: Disposal,
}

impl Bed {
    fn start(kind: Kind, name: &str, uid: &str, settings: LogSettings, pods: Vec<Pod>) -> Self {
        let tasks = Tasks::new(Handle::current(), |report: TaskReport| panic!("a task ended: {report}"));
        let disposal = Disposal::start(&tasks);
        let pods = Store::new(&disposal, pods);
        let others =
            (Store::new(&disposal, Vec::new()), Store::new(&disposal, Vec::new()), Store::new(&disposal, Vec::new()));
        let owners = TestOwners::start(
            &tasks,
            disposal.bin(),
            others.0.reader(),
            others.1.reader(),
            pods.reader(),
            others.2.reader(),
        );
        let (script, calls) = Script::new();
        let notice = Notice::new();
        let parts = SessionParts {
            tasks: tasks.clone(),
            pods: pods.reader(),
            owners: owners.reader(),
            signal: Signal::new(),
            clock: WallClock::new(|| SystemTime::UNIX_EPOCH),
            settings,
            restart: Backoff::default(),
            bin: disposal.bin(),
            notice: notice.clone(),
        };
        let request = LogRequest {
            cluster: ClusterId::new("ams"),
            kind,
            key: ObjectKey::new("shop", name),
            uid: uid.into(),
            options: LogOptions::default(),
        };
        let session = LogSession::start_with(request, parts, script);
        Self { session, calls, changes: notice.changes(), pods, _owners: owners, _others: others, _disposal: disposal }
    }

    /// The next opens, in the sequence of their pods.
    async fn calls(&mut self, count: usize) -> Vec<Call> {
        let mut calls = Vec::new();
        for _ in 0..count {
            calls.push(timeout(LIMIT, self.calls.recv()).await.expect("in time").expect("an open"));
        }
        calls.sort_by(|a, b| a.pod.cmp(&b.pod));
        calls
    }

    async fn until(&mut self, what: &str, met: impl Fn(&LogView) -> bool) -> Held<LogView> {
        let wait = async {
            loop {
                let view = self.session.view();
                if met(&view) {
                    return view;
                }
                self.changes.next().await;
            }
        };
        timeout(LIMIT, wait).await.unwrap_or_else(|_| panic!("in time: {what}: {:?}", *self.session.view()))
    }
}

const LIMIT: Duration = Duration::from_secs(60);

/// The texts of the lines of a view, with a mark for a late line.
fn texts(view: &LogView) -> Vec<String> {
    let lines = &view.lines;
    lines
        .range(lines.first(), lines.end())
        .map(|(_, line)| format!("{}{}", line.text, if line.late { " late" } else { "" }))
        .collect()
}

fn owned(name: &str, uid: &str) -> Pod {
    pod(name, uid, json!({"running": {}}), |pod| {
        pod["status"]["containerStatuses"][0]["containerID"] = json!(format!("containerd://{uid}"));
        pod["metadata"]["ownerReferences"] =
            json!([{"apiVersion": "apps/v1", "kind": "ReplicaSet", "name": "web", "uid": "rs1", "controller": true}]);
    })
}

#[tokio::test(start_paused = true)]
async fn a_pod_view_reads_the_container_that_restarts_and_shows_a_gone_pod() {
    let restarts = pod("web-1", "u1", json!({"running": {}}), |pod| {
        pod["status"]["containerStatuses"][0]["containerID"] = json!("containerd://a1");
        let side = json!({"name": "side", "image": "s", "imageID": "", "ready": true, "restartCount": 2,
            "containerID": "containerd://s3", "state": {"running": {}}});
        pod["status"]["containerStatuses"].as_array_mut().expect("a list").push(side);
    });
    let mut bed = Bed::start(Kind::Pod, "web-1", "u1", LogSettings::default(), vec![restarts]);
    let call = bed.calls(1).await.pop().expect("one open");
    assert_eq!(call.container, "side");
    let stream = call.open();
    stream.send(data(&[(1.0, "one"), (2.0, "two")])).expect("open");
    let view = bed.until("two lines", |view| view.lines.len() == 2).await;
    assert_eq!((texts(&view), view.pods.clone()), (vec!["one".to_owned(), "two".to_owned()], PodsState::Following));
    assert_eq!((view.sources[0].container.as_ref(), view.sources[0].lines), ("side", 2));

    bed.pods.set(Vec::new());
    bed.until("the pod is gone", |view| view.pods == PodsState::Gone).await;
    drop(stream);
    let ended = SourceState::Ended { reason: "the pod is gone".into() };
    bed.until("the end of the source", |view| view.sources[0].state == ended).await;
}

#[tokio::test(start_paused = true)]
async fn a_workload_view_has_the_lines_of_its_pods_in_time_and_follows_the_pods_that_come_and_go() {
    let mut bed = Bed::start(
        Kind::ReplicaSet,
        "web",
        "rs1",
        LogSettings::default(),
        vec![owned("web-1", "u1"), owned("web-2", "u2")],
    );
    let mut calls = bed.calls(2).await.into_iter().map(Call::open);
    let (one, two) = (calls.next().expect("web-1"), calls.next().expect("web-2"));
    one.send(data(&[(1.0, "a"), (3.0, "c")])).expect("open");
    two.send(data(&[(2.0, "b")])).expect("open");
    let view = bed.until("three lines", |view| view.lines.len() == 3).await;
    assert_eq!(texts(&view), ["a", "b", "c"], "the hold of the merge puts each line at its place");

    bed.pods.set(vec![owned("web-1", "u1"), owned("web-2", "u2"), owned("web-3", "u3")]);
    let three = bed.calls(1).await.pop().expect("the new pod").open();
    three.send(data(&[(4.0, "d")])).expect("open");
    two.send(data(&[(3.5, "late")])).expect("open");
    let view = bed.until("five lines", |view| view.lines.len() == 5).await;
    assert_eq!(texts(&view), ["a", "b", "c", "late", "d"]);
    let pods: Vec<&str> = view.sources.iter().map(|source| source.pod.name.as_ref()).collect();
    assert_eq!(pods.len(), 3);

    bed.pods.set(vec![owned("web-2", "u2"), owned("web-3", "u3")]);
    drop(one);
    let view = bed.until("the end of web-1", |view| view.sources.iter().any(|source| source.state.is_final())).await;
    let ended = view.sources.iter().find(|source| source.state.is_final()).expect("one end");
    assert_eq!((ended.pod.name.as_ref(), ended.state.to_string().as_str()), ("web-1", "ended: the pod is gone"));
    two.send(data(&[(5.0, "e")])).expect("open");
    three.send(data(&[(6.0, "f")])).expect("open");
    let view = bed.until("seven lines", |view| view.lines.len() == 7).await;
    assert_eq!(texts(&view)[5..], ["e", "f"], "an ended source holds nobody");
}

#[tokio::test(start_paused = true)]
async fn a_view_has_its_most_streams_and_a_source_that_ends_frees_its_stream() {
    let settings = LogSettings { streams: 2, ..LogSettings::default() };
    let pods = vec![owned("web-1", "u1"), owned("web-2", "u2"), owned("web-3", "u3")];
    let mut bed = Bed::start(Kind::ReplicaSet, "web", "rs1", settings, pods);
    let mut calls = bed.calls(2).await;
    bed.until("one container with no stream", |view| view.unstreamed == 1 && view.sources.len() == 2).await;
    let refused = calls.remove(0);
    refused.refuse(CoreError::answered(403, "forbidden"));
    let third = bed.calls(1).await.pop().expect("the third pod");
    assert_eq!(third.pod.name.as_ref(), "web-3");
    let view = bed.until("each container has a stream", |view| view.unstreamed == 0 && view.sources.len() == 3).await;
    assert!(matches!(view.sources[0].state, SourceState::Failed { .. }), "{:?}", view.sources[0]);
}

#[tokio::test(start_paused = true)]
async fn one_million_lines_keep_the_ring_of_a_view_at_its_limit() {
    let mut bed = Bed::start(Kind::Pod, "web-1", "u1", LogSettings::default(), vec![owned("web-1", "u1")]);
    let stream = bed.calls(1).await.pop().expect("one open").open();
    const CHUNK: usize = 10_000;
    for chunk in 0..100 {
        let mut text = String::with_capacity(CHUNK * 48);
        for number in chunk * CHUNK..(chunk + 1) * CHUNK {
            text.push_str(&format!("1970-01-01T00:00:01.{number:09}Z line {number}\n"));
        }
        stream.send(Ok(Bytes::from(text))).expect("open");
    }
    let limits = LogSettings::default().ring;
    let view = bed
        .until("each line", |view| {
            assert!(view.lines.len() <= limits.lines && view.bytes <= limits.bytes, "the ring is inside its limits");
            view.sources.first().is_some_and(|source| source.lines == 1_000_000) && view.lines.end() == 1_000_000
        })
        .await;
    assert_eq!((view.lines.len(), view.lines.first(), view.dropped), (200_000, 800_000, 800_000));
    let last = view.lines.get(999_999).expect("the last line");
    assert_eq!(last.text.as_ref(), "line 999999");
    assert!(view.bytes < 200_000 * 100, "{} bytes for 200,000 lines", view.bytes);
}
