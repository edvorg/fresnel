//! The drain of a node at its worst moments (research 08 section 7.2): a list with a pod of
//! an other node, a list with no answer after the cordon, a conflict for a pod that is still
//! there, a cancel after the server took an eviction, and a node with 500 pods. The fake
//! server drops the field selector of the list, as a broken proxy would. The test publishes
//! the store of the pods itself, as the watch would.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use fresnel_core::cluster::ClusterId;
use fresnel_core::kinds::Kind;
use fresnel_core::net::{ErrorClass, Limits, NetSettings};
use fresnel_core::ops::drain::{Drain, DrainOptions, PodState};
use fresnel_core::ops::{self, Cancel, JobState, Operation, Order, Outcome, Parts, Progress, Report, Target};
use fresnel_core::published::{Disposal, Notice, Publisher};
use fresnel_core::store::{ObjectKey, Snapshot, StoreState};
use fresnel_testkit::Fault;
use http::Method;
use k8s_openapi::api::core::v1::Pod;
use serde_json::json;
use tokio::sync::mpsc;

use crate::bed::{Bed, CONTEXT};

const NODE: &str = "/api/v1/nodes/n1";
const PODS: &str = "/api/v1/pods";
const WEB: &str = "/api/v1/namespaces/shop/pods/web-1/eviction";
/// The longest time of one drain of the test: a refusal costs the pause of 5 s.
const LIMIT: Duration = Duration::from_secs(30);

/// The store of the pods of the cluster. The publisher drops before the thread of its bin.
struct Pods {
    publisher: Publisher<Snapshot<Pod>>,
    _disposal: Disposal,
}

impl Pods {
    fn start(bed: &Bed, pods: Vec<Pod>) -> Self {
        let disposal = Disposal::start(&bed.tasks.tasks);
        Self { publisher: Publisher::new(snapshot(pods), disposal.bin(), Notice::new()), _disposal: disposal }
    }
}

fn snapshot(pods: Vec<Pod>) -> Snapshot<Pod> {
    let now = SystemTime::now();
    let items = pods.into_iter().map(Arc::new).collect();
    Snapshot {
        generation: 1,
        state: StoreState::Live,
        since: now,
        last_contact: Some(now),
        listed: true,
        dropped: 0,
        items,
    }
}

/// A pod of a ReplicaSet on a node, with the UID `uid-<name>`.
fn pod(name: &str, node: &str) -> Pod {
    let owner = json!({"apiVersion": "apps/v1", "kind": "ReplicaSet", "name": "web", "uid": "rs", "controller": true});
    let meta = json!({"name": name, "namespace": "shop", "uid": format!("uid-{name}"), "ownerReferences": [owner]});
    let pod = json!({"metadata": meta, "spec": {"nodeName": node, "containers": [{"name": "c"}]},
        "status": {"phase": "Running"}});
    serde_json::from_value(pod).expect("a pod")
}

/// The answer to the list of the pods: these pods, whatever the field selector.
fn list(bed: &Bed, pods: &[Pod]) {
    let list = json!({"kind": "PodList", "apiVersion": "v1", "metadata": {"resourceVersion": "1"}, "items": pods});
    bed.server.set_body(PODS, &list.to_string());
}

fn sent(bed: &Bed, method: &Method, path: impl Fn(&str) -> bool) -> usize {
    bed.server.requests().iter().filter(|seen| seen.method == *method && path(seen.uri.path())).count()
}

fn evictions(bed: &Bed) -> usize {
    sent(bed, &Method::POST, |path| path.ends_with("/eviction"))
}

/// Gives each report of the job to the test.
struct Forward(Option<mpsc::UnboundedSender<Report>>);

impl Progress for Forward {
    fn show(&mut self, report: &Report) {
        if self.0.as_ref().is_some_and(|sender| sender.send(report.clone()).is_err()) {
            // The test stopped to read: no more reports go to it.
            self.0 = None;
        }
    }
}

/// The pods of the drain of a report, with no pods before the list.
fn drained(report: &Report) -> Option<&Drain> {
    match &report.outcome {
        Outcome::Drain(drain) => Some(drain),
        _ => None,
    }
}

/// Drains the node `n1` with these options. `each` sees each report. Gives the last report.
async fn drain(bed: &Bed, pods: &Pods, options: DrainOptions, cancel: Cancel, mut each: impl FnMut(&Report)) -> Report {
    let key = ObjectKey::new("", "n1");
    let target = Target { cluster: ClusterId::new(CONTEXT), kind: Kind::Node, key, uid: "uid-n1".into() };
    let order = Order::new(target, Operation::Drain(options)).expect("an order");
    let gateway = bed.link.gateway.clone();
    let reader = Some(pods.publisher.reader());
    let parts =
        Parts { gateway, deployments: None, stateful_sets: None, daemon_sets: None, jobs: None, pods: reader, cancel };
    let (sender, mut reports) = mpsc::unbounded_channel();
    let run = async move { ops::run(&order, parts, &mut Forward(Some(sender))).await };
    let follow = async move {
        let mut last = Report::default();
        while let Some(report) = reports.recv().await {
            each(&report);
            last = report;
        }
        last
    };
    let (state, last) = tokio::time::timeout(LIMIT, async { tokio::join!(run, follow) }).await.expect("in time");
    assert_eq!(state, last.state, "the last report has the final state");
    last
}

/// The states of the pods of the last report.
fn states(report: &Report) -> Vec<PodState> {
    drained(report).map(|drain| drain.pods.iter().map(|pod| pod.state.clone()).collect()).unwrap_or_default()
}

/// A server that drops the field selector lists each pod of the cluster: the drain of one
/// node would evict them all.
#[tokio::test]
async fn a_list_with_a_pod_of_an_other_node_stops_the_drain_before_its_first_eviction() {
    let bed = Bed::start(NetSettings::default()).await;
    let here = pod("web-1", "n1");
    list(&bed, &[here.clone(), pod("db-1", "n2")]);
    let pods = Pods::start(&bed, vec![here]);
    let report = drain(&bed, &pods, DrainOptions::default(), Cancel::pair().1, |_| {}).await;
    let JobState::Failed(error) = &report.state else { panic!("a refusal: {report:#?}") };
    assert!(error.to_string().contains("1 pod of an other node"), "{error}");
    assert_eq!(evictions(&bed), 0, "no eviction went out");
    assert!(drained(&report).is_some_and(|drain| drain.cordoned), "the node stays cordoned: {report:#?}");
}

/// The cordon is in effect when the list fails: the job is no write that did not go out.
#[tokio::test]
async fn a_list_with_no_answer_after_the_cordon_fails_the_drain_and_tells_the_cordon() {
    let limits =
        Limits { list_idle: Duration::from_millis(500), list_total: Duration::from_secs(1), ..Limits::default() };
    let bed = Bed::start(NetSettings { limits, ..NetSettings::default() }).await;
    bed.server.add_fault(Fault::no_answer().on_path(PODS));
    let pods = Pods::start(&bed, Vec::new());
    let report = drain(&bed, &pods, DrainOptions::default(), Cancel::pair().1, |_| {}).await;
    let JobState::Failed(error) = &report.state else { panic!("failed, not \"not sent\": {report:#?}") };
    assert_eq!(error.class(), ErrorClass::Timeout, "{error}");
    assert!(drained(&report).is_some_and(|drain| drain.cordoned), "{report:#?}");
    assert_eq!(sent(&bed, &Method::PATCH, |path| path == NODE), 1, "one cordon");
}

/// HTTP 409 means that the pod of the list is gone and a new pod has its name, as the
/// eviction carries the UID of the pod. A conflict for a pod that the store still shows is
/// a refusal for now: the eviction goes again after the pause, and the pod counts as gone
/// only when it is.
#[tokio::test]
async fn a_conflict_for_a_pod_that_the_store_still_shows_sends_the_eviction_again() {
    let bed = Bed::start(NetSettings::default()).await;
    let web = pod("web-1", "n1");
    list(&bed, std::slice::from_ref(&web));
    let pods = Pods::start(&bed, vec![web.clone()]);
    bed.server.add_fault(Fault::conflict().on_path(WEB).for_time(Duration::from_secs(2)));
    let mut refused = false;
    let report = drain(&bed, &pods, DrainOptions::default(), Cancel::pair().1, |report| {
        let states = states(report);
        refused |= matches!(states.first(), Some(PodState::Blocked { reason, .. }) if reason.contains("HTTP 409"));
        if states.first() == Some(&PodState::Terminating) {
            pods.publisher.publish(snapshot(Vec::new()));
        }
    })
    .await;
    assert_eq!((&report.state, states(&report)), (&JobState::Done, vec![PodState::Gone]), "{report:#?}");
    assert!(refused, "the pod showed the conflict");
    assert_eq!(evictions(&bed), 2, "the eviction went again after the conflict");

    // The store has a new pod of the name: the pod of the list is gone.
    let mut new = web;
    new.metadata.uid = Some("uid-new".to_owned());
    let pods = Pods::start(&bed, vec![new]);
    bed.server.add_fault(Fault::conflict().on_path(WEB));
    let report = drain(&bed, &pods, DrainOptions::default(), Cancel::pair().1, |_| {}).await;
    assert_eq!((&report.state, states(&report)), (&JobState::Done, vec![PodState::Gone]), "{report:#?}");
    assert_eq!(evictions(&bed), 3, "one eviction");
}

/// The server took the eviction: the pod ends, also after the cancel of the drain.
#[tokio::test]
async fn a_cancel_after_the_server_took_an_eviction_leaves_the_pod_terminating() {
    let bed = Bed::start(NetSettings::default()).await;
    let web = pod("web-1", "n1");
    list(&bed, std::slice::from_ref(&web));
    let pods = Pods::start(&bed, vec![web]);
    let (cancel, signal) = Cancel::pair();
    let report = drain(&bed, &pods, DrainOptions::default(), signal, |report| {
        if states(report).first() == Some(&PodState::Terminating) {
            cancel.send_replace(true);
        }
    })
    .await;
    assert_eq!((&report.state, states(&report)), (&JobState::Cancelled, vec![PodState::Terminating]), "{report:#?}");
}

/// Each pod has its own task: 500 evictions go out one time each, and the drain ends when
/// the store shows each pod gone.
#[tokio::test]
async fn a_node_with_500_pods_evicts_each_one_one_time() {
    let bed = Bed::start(NetSettings::default()).await;
    let many: Vec<Pod> = (0..500).map(|index| pod(&format!("web-{index}"), "n1")).collect();
    list(&bed, &many);
    let pods = Pods::start(&bed, many);
    let mut emptied = false;
    let report = drain(&bed, &pods, DrainOptions::default(), Cancel::pair().1, |report| {
        let all = states(report);
        if !emptied && all.len() == 500 && all.iter().all(|state| *state == PodState::Terminating) {
            emptied = true;
            pods.publisher.publish(snapshot(Vec::new()));
        }
    })
    .await;
    assert_eq!(report.state, JobState::Done, "{report:#?}");
    assert!(states(&report).iter().all(|state| *state == PodState::Gone));
    assert_eq!(evictions(&bed), 500);
}
