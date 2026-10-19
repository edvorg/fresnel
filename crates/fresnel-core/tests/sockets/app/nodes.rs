//! The cordon and the drain of a node through the core (plan section 10): `Command::Run`
//! with the order of a Node starts a job in its cluster, and the drain reads the store of the
//! pods of the core for the end of each pod that it evicts. The cluster tests `k3s_ops` and
//! `k3s_drain` send the same orders to k3s; this one runs in each check, with no Docker.

use std::sync::Arc;

use fresnel_core::app::{Command, CoreSettings, CoreView, Health, JobView};
use fresnel_core::cluster::ClusterId;
use fresnel_core::journal::TASK_SOURCE;
use fresnel_core::kinds::Kind;
use fresnel_core::ops::drain::{DrainOptions, PodState};
use fresnel_core::ops::{JobState, Operation, Order, Outcome, Target};
use fresnel_core::store::ObjectKey;
use fresnel_testkit::FakeApiServer;
use http::Method;

use super::keys::requests;
use super::{App, home, write_kubeconfig};

const NODE: &str = "/api/v1/nodes/n1";
const PODS: &str = "/api/v1/pods";
const EVICTION: &str = "/api/v1/namespaces/shop/pods/web-1/eviction";

/// The pod `web-1` of a ReplicaSet on the node `n1`.
const WEB: &str = r#"{"apiVersion":"v1","kind":"Pod","metadata":{"name":"web-1","namespace":"shop","uid":"uid-web-1",
"resourceVersion":"11","ownerReferences":[{"apiVersion":"apps/v1","kind":"ReplicaSet","name":"web","uid":"rs",
"controller":true}]},"spec":{"nodeName":"n1","containers":[{"name":"c"}]},"status":{"phase":"Running"}}"#;

fn order(operation: Operation) -> Order {
    let target =
        Target { cluster: ClusterId::new("ams"), kind: Kind::Node, key: ObjectKey::new("", "n1"), uid: "n".into() };
    Order::new(target, operation).expect("an order")
}

fn job(view: &CoreView, number: u64) -> Option<&JobView> {
    view.jobs.jobs.iter().map(Arc::as_ref).find(|job| job.number == number)
}

fn ended(number: u64) -> impl Fn(&CoreView) -> bool {
    move |view| job(view, number).is_some_and(|job| job.report.state.is_final())
}

/// The states of the pods of the drain of a job.
fn pods(view: &CoreView, number: u64) -> Vec<PodState> {
    match job(view, number).map(|job| &job.report.outcome) {
        Some(Outcome::Drain(drain)) => drain.pods.iter().map(|pod| pod.state.clone()).collect(),
        _ => Vec::new(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cordon_and_a_drain_run_as_jobs_and_the_drain_ends_when_the_store_of_the_core_loses_the_pod() {
    let server = FakeApiServer::start().await.expect("a server");
    let web = WEB.replace('\n', "");
    server.set_body(PODS, &format!(r#"{{"kind":"PodList","metadata":{{"resourceVersion":"11"}},"items":[{web}]}}"#));
    server.set_body(
        NODE,
        r#"{"kind":"Node","metadata":{"name":"n1","resourceVersion":"12"},"spec":{"unschedulable":true}}"#,
    );
    let home = home("nodes");
    write_kubeconfig(&home, "ams", &[("ams", server.addr(), &server, false)]);
    let mut app = App::started(&home, CoreSettings::default());
    app.until_health("ams", Health::Live).await;
    let pod_count = |view: &CoreView| {
        let data = view.clusters.iter().find(|row| row.id.as_str() == "ams").and_then(|row| row.data.as_ref());
        data.filter(|data| data.pods.listed && data.pods.state.is_live()).map(|data| data.pods.items.len())
    };
    app.until("the pod of the node in the store", |view| pod_count(view) == Some(1)).await;

    app.core.send(Command::Run(vec![order(Operation::Cordon)]));
    let view = app.until("the cordon", ended(1)).await;
    assert_eq!(job(&view, 1).map(|job| &job.report.state), Some(&JobState::Done), "{:#?}", job(&view, 1));
    assert_eq!(requests(&server, &Method::PATCH, NODE), 1);

    // The drain cordons again, lists the pods of the node and evicts web-1. Its job ends when
    // the watch of the store of the core takes the pod away.
    app.core.send(Command::Run(vec![order(Operation::Drain(DrainOptions::default()))]));
    app.until("the eviction", |view| pods(view, 2) == [PodState::Terminating]).await;
    assert_eq!(requests(&server, &Method::POST, EVICTION), 1);
    assert!(!ended(2)(&app.core.view()), "the drain waits for the end of the pod");
    let deleted = format!(r#"{{"type":"DELETED","object":{web}}}"#);
    assert!(server.send_event(PODS, &deleted) > 0, "the watch of the pods");
    let view = app.until("the end of the drain", ended(2)).await;
    let drain = job(&view, 2).expect("the job");
    assert_eq!((&drain.report.state, pods(&view, 2)), (&JobState::Done, vec![PodState::Gone]), "{drain:#?}");
    assert_eq!((requests(&server, &Method::PATCH, NODE), requests(&server, &Method::POST, EVICTION)), (2, 1));

    let ends = app.until_texts(TASK_SOURCE, |texts| texts.len() == 2).await;
    assert!(ends.iter().all(|text| text == "task job [ams] ended; no restart"), "{ends:?}");
    app.expected_task_ends = 2;
}
