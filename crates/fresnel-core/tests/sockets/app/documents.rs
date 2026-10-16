//! The create of the documents of a text in turn through `Command::RunInTurn`, as the YAML
//! tab sends them (plan section 12.9). The cluster test `k3s_app` sends the same command to
//! k3s; this runs in each check, with no Docker.

use std::sync::Arc;
use std::time::Duration;

use fresnel_core::app::{Command, CoreSettings, CoreView, Health, JobView};
use fresnel_core::cluster::ClusterId;
use fresnel_core::journal::TASK_SOURCE;
use fresnel_core::kinds::Kind;
use fresnel_core::manifest::documents::documents;
use fresnel_core::net::{Limits, NetSettings};
use fresnel_core::ops::{JobState, Operation, Order, StepState, Target};
use fresnel_core::store::ObjectKey;
use fresnel_testkit::{FakeApiServer, Fault};
use http::Method;

use super::{App, home, write_kubeconfig};

const CONFIG_MAPS: &str = "/api/v1/namespaces/shop/configmaps";

/// The create of a ConfigMap of this name in the namespace `shop` of ams.
fn create(name: &str) -> Order {
    let text = format!("apiVersion: v1\nkind: ConfigMap\nmetadata: {{name: {name}}}\n");
    let document = documents(&text).expect("valid YAML").remove(0).expect("an object");
    let key = ObjectKey::new("shop", name);
    let target = Target { cluster: ClusterId::new("ams"), kind: Kind::ConfigMap, key, uid: "".into() };
    Order::new(target, Operation::Create(document)).expect("an order")
}

/// The create of the second document waits for the end of the first one, also when that one
/// ends with a result that is not known. A fault holds each create of its first second: the
/// second create, which comes after the time limit of the first one, gets its answer.
#[tokio::test(flavor = "multi_thread")]
async fn each_document_waits_for_the_end_of_the_one_before_it() {
    let ams = FakeApiServer::start().await.expect("a server");
    ams.set_body(CONFIG_MAPS, r#"{"apiVersion":"v1","kind":"ConfigMap","metadata":{"name":"two","namespace":"shop"}}"#);
    let home = home("documents");
    write_kubeconfig(&home, "ams", &[("ams", ams.addr(), &ams, false)]);
    let limits = Limits { write: Duration::from_secs(2), ..Limits::default() };
    let settings = CoreSettings { net: NetSettings { limits, ..NetSettings::default() }, ..CoreSettings::default() };
    let mut app = App::started(&home, settings);
    app.until_health("ams", Health::Live).await;

    ams.add_fault(Fault::no_answer().on_path(CONFIG_MAPS).for_time(Duration::from_secs(1)));
    app.core.send(Command::RunInTurn(vec![create("one"), create("two")]));
    let ended =
        |view: &CoreView| view.jobs.jobs.len() == 2 && view.jobs.jobs.iter().all(|job| job.report.state.is_final());
    let view = app.until("the end of both jobs", ended).await;
    let jobs: Vec<&JobView> = view.jobs.jobs.iter().map(Arc::as_ref).collect();
    assert!(matches!(jobs[0].report.state, JobState::Unknown(_)), "{:?}", jobs[0].report);
    assert_eq!(jobs[1].report.state, JobState::Done, "{:?}", jobs[1].report);
    let wait = format!("wait for the end of job {}", jobs[0].number);
    let first = jobs[1].report.steps.first().map(|step| (step.text.as_str(), &step.state));
    assert_eq!(first, Some((wait.as_str(), &StepState::Done(format!("job {} ended", jobs[0].number)))));
    let posts = ams.requests().into_iter().filter(|seen| seen.method == Method::POST).count();
    assert_eq!(posts, 2, "one create for each document");
    let ends = app.until_texts(TASK_SOURCE, |texts| texts.len() == 2).await;
    assert!(ends.iter().all(|text| text == "task job [ams] ended; no restart"), "{ends:?}");
    app.expected_task_ends = 2;
}
