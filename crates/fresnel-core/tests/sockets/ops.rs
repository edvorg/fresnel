//! The fates of the writes of the operations on the fake API server (design rule 6).

use std::time::Duration;

use fresnel_core::cluster::ClusterId;
use fresnel_core::kinds::Kind;
use fresnel_core::net::{Limits, NetSettings};
use fresnel_core::ops::{self, Cancel, JobState, Operation, Order, Parts, Progress, Report, StepState, Target};
use fresnel_core::store::ObjectKey;
use fresnel_testkit::Fault;
use http::Method;

use crate::bed::{Bed, CONTEXT};

const SCALE: &str = "/apis/apps/v1/namespaces/shop/deployments/web/scale";
const WEB: &str = "/apis/apps/v1/namespaces/shop/deployments/web";

#[derive(Default)]
struct Last(Report);

impl Progress for Last {
    fn show(&mut self, report: &Report) {
        self.0 = report.clone();
    }
}

fn order(cluster: &str, operation: Operation) -> Order {
    let key = ObjectKey::new("shop", "web");
    let target = Target { cluster: ClusterId::new(cluster), kind: Kind::Deployment, key, uid: "u1".into() };
    Order::new(target, operation).expect("an order")
}

fn parts(bed: &Bed) -> Parts {
    let gateway = bed.link.gateway.clone();
    let cancel = Cancel::pair().1;
    Parts { gateway, deployments: None, stateful_sets: None, daemon_sets: None, jobs: None, pods: None, cancel }
}

async fn run(bed: &Bed, order: &Order) -> Report {
    let mut last = Last::default();
    let state = ops::run(order, parts(bed), &mut last).await;
    assert_eq!(state, last.0.state);
    last.0
}

fn sent(bed: &Bed, method: &Method, path: &str) -> usize {
    bed.server.requests().iter().filter(|seen| seen.method == *method && seen.uri.path() == path).count()
}

#[tokio::test]
async fn a_write_with_no_answer_has_an_unknown_result_goes_out_one_time_and_the_object_is_read() {
    let limits = Limits { write: Duration::from_millis(500), ..Limits::default() };
    let bed = Bed::start(NetSettings { limits, ..NetSettings::default() }).await;
    bed.server.add_fault(Fault::no_answer().on_path(SCALE));
    bed.server.set_body(WEB, r#"{"metadata":{"name":"web","namespace":"shop","uid":"u1","resourceVersion":"88"}}"#);

    let report = run(&bed, &order(CONTEXT, Operation::Scale(3))).await;
    let JobState::Unknown(error) = &report.state else { panic!("unknown: {report:#?}") };
    assert_eq!(error.class(), fresnel_core::net::ErrorClass::Timeout, "{error}");
    let steps: Vec<(&str, &StepState)> = report.steps.iter().map(|step| (step.text.as_str(), &step.state)).collect();
    let read = StepState::Done("the object exists, version 88".into());
    let expected = [
        ("set the count of pods to 3", &StepState::Failed),
        ("read the object, because the result of the write is not known", &read),
    ];
    assert_eq!(steps, expected);
    assert_eq!(sent(&bed, &Method::PATCH, SCALE), 1, "the write went out one time");
}

#[tokio::test]
async fn a_refusal_of_the_server_fails_with_its_message_and_an_other_cluster_gets_no_request() {
    let bed = Bed::start(NetSettings::default()).await;
    bed.server.add_fault(Fault::forbidden().on_path(SCALE));
    let report = run(&bed, &order(CONTEXT, Operation::Scale(3))).await;
    let JobState::Failed(error) = &report.state else { panic!("failed: {report:#?}") };
    assert_eq!(error.status(), Some(403));
    assert!(error.to_string().starts_with("forbidden: the user of the test cannot read this path"), "{error}");
    assert_eq!(report.steps.len(), 1, "a refusal needs no read: nothing changed");

    let report = run(&bed, &order("other", Operation::Scale(3))).await;
    let JobState::NotSent(error) = &report.state else { panic!("not sent: {report:#?}") };
    assert_eq!(error.to_string(), "a request for other cannot go to bed");
    assert_eq!(sent(&bed, &Method::PATCH, SCALE), 1, "only the first order went out");
}
