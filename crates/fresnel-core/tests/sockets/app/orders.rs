//! The jobs and the permissions in the core: `Command::Run` starts a job in the cluster of
//! each order, `Command::Cancel` ends a wait of a job, and `Command::ReadRules` and
//! `Command::CheckAccess` give the permissions that a view decides with. The cluster test
//! `k3s_app` sends the same commands to k3s; these run in each check, with no Docker.

use std::sync::Arc;

use fresnel_core::app::{AccessView, CheckState, Command, CoreSettings, CoreView, Health, JobView, RulesState};
use fresnel_core::cluster::ClusterId;
use fresnel_core::config::{Edit, EditValue};
use fresnel_core::journal::TASK_SOURCE;
use fresnel_core::keys::{NewSecret, SecretType};
use fresnel_core::kinds::{Action, Kind};
use fresnel_core::ops::access::{self, Decision};
use fresnel_core::ops::{JobState, Operation, Order, Outcome, Rollout, Target};
use fresnel_core::store::ObjectKey;
use fresnel_testkit::FakeApiServer;
use http::Method;
use k8s_openapi::api::apps::v1::Deployment;

use super::keys::requests;
use super::{App, home, write_kubeconfig};

const DEPLOYMENTS: &str = "/apis/apps/v1/deployments";
const SCALE: &str = "/apis/apps/v1/namespaces/shop/deployments/web/scale";
const RULES: &str = "/apis/authorization.k8s.io/v1/selfsubjectrulesreviews";
const ACCESS: &str = "/apis/authorization.k8s.io/v1/selfsubjectaccessreviews";

/// The Deployment `web` with this version. Its rollout waits for the second pod.
fn web(version: u32) -> String {
    format!(
        r#"{{"apiVersion":"apps/v1","kind":"Deployment","metadata":{{"name":"web","namespace":"shop","uid":"u-web",
        "resourceVersion":"{version}","generation":1}},"spec":{{"replicas":2,"selector":{{}},"template":{{}}}},
        "status":{{"observedGeneration":1,"replicas":2,"updatedReplicas":1}}}}"#
    )
}

/// The two clusters `ams` and `fra`, both selected and live, with the Deployment `web`.
async fn started(label: &str) -> (App, FakeApiServer, FakeApiServer, fresnel_testkit::Home) {
    let (ams, fra) = (FakeApiServer::start().await.expect("a server"), FakeApiServer::start().await.expect("a server"));
    for server in [&ams, &fra] {
        server.set_body(DEPLOYMENTS, &format!(r#"{{"metadata":{{"resourceVersion":"10"}},"items":[{}]}}"#, web(10)));
    }
    let home = home(label);
    write_kubeconfig(&home, "ams", &[("ams", ams.addr(), &ams, false), ("fra", fra.addr(), &fra, false)]);
    let app = App::started(&home, CoreSettings::default());
    app.select("fra", true);
    app.until_health("ams", Health::Live).await;
    app.until_health("fra", Health::Live).await;
    (app, ams, fra, home)
}

fn order(cluster: &str, operation: Operation) -> Order {
    let key = ObjectKey::new("shop", "web");
    let target = Target { cluster: ClusterId::new(cluster), kind: Kind::Deployment, key, uid: "u-web".into() };
    Order::new(target, operation).expect("an order")
}

fn job(view: &CoreView, number: u64) -> Option<&JobView> {
    view.jobs.jobs.iter().map(Arc::as_ref).find(|job| job.number == number)
}

#[tokio::test(flavor = "multi_thread")]
async fn an_order_runs_as_a_job_of_its_cluster_and_a_cancel_ends_its_wait() {
    let (mut app, ams, fra, _home) = started("orders").await;
    app.core.send(Command::ShowKinds([Kind::Deployment].into()));
    let deployments = |view: &CoreView| {
        let row = view.clusters.iter().find(|row| row.id.as_str() == "ams");
        row.and_then(|row| row.data.as_ref()).and_then(|data| data.snapshot::<Deployment>())
    };
    // A live store has its watch open, so the event of the write below cannot be lost.
    let live = |view: &CoreView| deployments(view).is_some_and(|store| store.listed && store.state.is_live());
    let shown = app.until("the watch of the Deployments of ams", live).await;
    let before = deployments(&shown).expect("the store");

    // The scale goes to ams only, and its answer gives the mark until the watch event.
    ams.set_body(
        SCALE,
        r#"{"kind":"Scale","metadata":{"name":"web","namespace":"shop","uid":"u-web","resourceVersion":"11"}}"#,
    );
    app.core.send(Command::Run(vec![order("ams", Operation::Scale(3))]));
    let done =
        app.until("the job of the scale", |view| job(view, 1).is_some_and(|job| job.report.state.is_final())).await;
    let scaled = job(&done, 1).expect("the job");
    assert_eq!(scaled.report.state, JobState::Done, "{scaled:#?}");
    let Outcome::Written(Some(written)) = &scaled.report.outcome else { panic!("the answer: {scaled:#?}") };
    assert!(before.is_pending(written), "the row of web shows \"change pending\"");
    assert_eq!((requests(&ams, &Method::PATCH, SCALE), requests(&fra, &Method::PATCH, SCALE)), (1, 0));
    let event = format!(r#"{{"type":"MODIFIED","object":{}}}"#, web(11).replace('\n', ""));
    assert!(ams.send_event(DEPLOYMENTS, &event) > 0, "the watch of the Deployments");
    app.until("the watch event ends the mark", |view| {
        deployments(view).is_some_and(|store| !store.is_pending(written))
    })
    .await;

    // The rollout status of web waits for its second pod, until the cancel.
    app.core.send(Command::Run(vec![order("ams", Operation::RolloutStatus)]));
    let waits = |view: &CoreView| {
        job(view, 2).is_some_and(|job| matches!(job.report.outcome, Outcome::Rollout(Rollout::Waiting(_))))
    };
    app.until("the rollout status waits", waits).await;
    app.core.send(Command::Cancel(2));
    let ended = app.until("the cancel", |view| job(view, 2).is_some_and(|job| job.report.state.is_final())).await;
    assert_eq!(job(&ended, 2).map(|job| &job.report.state), Some(&JobState::Cancelled));
    assert_eq!(job(&ended, 1).map(|job| &job.report.state), Some(&JobState::Done), "the other job stays");

    // A job is a task that runs one time: the journal has the end of each one.
    let ends = app.until_texts(TASK_SOURCE, |texts| texts.len() == 2).await;
    assert!(ends.iter().all(|text| text == "task job [ams] ended; no restart"), "{ends:?}");
    app.expected_task_ends = 2;
}

/// The answer of a rules review with these rules.
fn rules(rules: &str) -> String {
    format!(
        r#"{{"kind":"SelfSubjectRulesReview","apiVersion":"authorization.k8s.io/v1","spec":{{}},
        "status":{{"resourceRules":{rules},"nonResourceRules":[],"incomplete":false}}}}"#
    )
}

fn access_of<'a>(view: &'a CoreView, cluster: &str) -> Option<&'a AccessView> {
    let row = view.clusters.iter().find(|row| row.id.as_str() == cluster)?;
    row.data.as_ref().map(|data| &*data.access)
}

#[tokio::test(flavor = "multi_thread")]
async fn the_rules_and_the_reviews_of_each_cluster_decide_its_actions() {
    let (app, ams, fra, _home) = started("permissions").await;
    ams.set_body(
        RULES,
        &rules(r#"[{"verbs":["get","list","watch"],"apiGroups":["apps"],"resources":["deployments"]}]"#),
    );
    fra.set_body(RULES, &rules(r#"[{"verbs":["*"],"apiGroups":["*"],"resources":["*"]}]"#));
    let review = |allowed: bool| {
        format!(
            r#"{{"kind":"SelfSubjectAccessReview","apiVersion":"authorization.k8s.io/v1","spec":{{}},
            "status":{{"allowed":{allowed},"reason":"no binding"}}}}"#
        )
    };
    ams.set_body(ACCESS, &review(false));
    fra.set_body(ACCESS, &review(true));

    for cluster in ["ams", "fra"] {
        app.core.send(Command::ReadRules { cluster: ClusterId::new(cluster), namespace: "shop".into() });
    }
    let read = |view: &CoreView| {
        let read = |cluster| {
            access_of(view, cluster).is_some_and(|access| matches!(access.rules.get("shop"), Some(RulesState::Read(_))))
        };
        read("ams") && read("fra")
    };
    let view = app.until("the rules of shop in each cluster", read).await;
    let web = ObjectKey::new("shop", "web");
    let decide =
        |cluster, action| access_of(&view, cluster).map(|access| access.decide(action, Kind::Deployment, &web));
    let off = "no rule lets you patch deployments/scale (apps) web in shop";
    assert_eq!(decide("ams", Action::Scale), Some(Decision::Denied(off.into())), "the scale is off with its reason");
    assert_eq!(decide("ams", Action::ViewManifest), Some(Decision::Allowed));
    assert_eq!(decide("fra", Action::Scale), Some(Decision::Allowed));

    // A Node has no namespace: a view asks the server about each request of the action.
    let node = ObjectKey::new("", "n1");
    let cordon = access::needs(Action::Cordon, Kind::Node, &node);
    for cluster in ["ams", "fra"] {
        for attributes in &cordon {
            app.core.send(Command::CheckAccess { cluster: ClusterId::new(cluster), attributes: attributes.clone() });
        }
    }
    let answered = |view: &CoreView| {
        let answered = |cluster| {
            let access = access_of(view, cluster);
            cordon.iter().all(|asked| {
                access.is_some_and(|access| matches!(access.checks.get(asked), Some(CheckState::Answered(_))))
            })
        };
        answered("ams") && answered("fra")
    };
    let view = app.until("the answers of the reviews", answered).await;
    let decide = |cluster| access_of(&view, cluster).map(|access| access.decide(Action::Cordon, Kind::Node, &node));
    let denied = "the server does not let you get nodes n1 in the cluster: no binding";
    assert_eq!(decide("ams"), Some(Decision::Denied(denied.into())));
    assert_eq!(decide("fra"), Some(Decision::Allowed));
    let reviews =
        |server: &FakeApiServer| (requests(server, &Method::POST, RULES), requests(server, &Method::POST, ACCESS));
    assert_eq!((reviews(&ams), reviews(&fra)), ((1, 2), (1, 2)), "each review went to its own cluster one time");
}

/// A read-only cluster of the settings gets no write from the core, also from a view that
/// did not close its gate: the job ends as not sent with the reason, and the server sees
/// no request. A read runs, the other cluster gets its write, and a save of the settings
/// view that ends the read-only state acts at once.
#[tokio::test(flavor = "multi_thread")]
async fn a_read_only_cluster_gets_no_write_and_a_read_and_the_other_cluster_go_on() {
    let (mut app, ams, fra, _home) = started("read-only").await;
    let ams_only = vec![Edit::new(&["clusters", "ams", "read_only"], EditValue::Flag(true))];
    app.core.send(Command::EditSettings(ams_only));
    app.until("the read-only cluster", |view| view.config.read_only(&ClusterId::new("ams"))).await;
    for server in [&ams, &fra] {
        server.set_body(SCALE, r#"{"kind":"Scale","metadata":{"name":"web","namespace":"shop","uid":"u-web"}}"#);
    }
    let orders =
        vec![order("ams", Operation::Scale(3)), order("fra", Operation::Scale(3)), order("ams", Operation::History)];
    app.core.send(Command::Run(orders));
    let ended = |numbers: &'static [u64]| {
        move |view: &CoreView| {
            numbers.iter().all(|number| job(view, *number).is_some_and(|job| job.report.state.is_final()))
        }
    };
    let view = app.until("the end of the three jobs", ended(&[1, 2, 3])).await;
    let refusal = "ams is read-only in the settings: the app makes no change in it.";
    assert!(
        matches!(job(&view, 1).map(|job| &job.report.state), Some(JobState::NotSent(error)) if error.to_string() == refusal),
        "{:#?}",
        job(&view, 1)
    );
    assert_eq!(job(&view, 2).map(|job| &job.report.state), Some(&JobState::Done), "the other cluster");
    assert!(!matches!(job(&view, 3).map(|job| &job.report.state), Some(JobState::NotSent(_))), "a read runs");
    assert_eq!((requests(&ams, &Method::PATCH, SCALE), requests(&fra, &Method::PATCH, SCALE)), (0, 1));

    let secret = NewSecret::new(ClusterId::new("ams"), "shop", "token", SecretType::Opaque);
    app.core.send(Command::CreateSecret(secret));
    let texts = app.until_texts("ams", |texts| texts.iter().any(|text| text.contains("did not go"))).await;
    assert!(texts.contains(&format!("the create of Secret shop/token did not go: {refusal}")), "{texts:#?}");
    assert_eq!(requests(&ams, &Method::POST, "/api/v1/namespaces/shop/secrets"), 0);

    app.core.send(Command::EditSettings(vec![Edit::new(&["clusters", "ams", "read_only"], EditValue::Flag(false))]));
    app.until("the end of the read-only state", |view| !view.config.read_only(&ClusterId::new("ams"))).await;
    app.core.send(Command::Run(vec![order("ams", Operation::Scale(3))]));
    let view = app.until("the scale after the change", ended(&[4])).await;
    assert_eq!(job(&view, 4).map(|job| &job.report.state), Some(&JobState::Done));
    assert_eq!(requests(&ams, &Method::PATCH, SCALE), 1);
    // Each job that went out is a task that runs one time; the refused one had no task.
    app.until_texts(TASK_SOURCE, |texts| texts.len() == 3).await;
    app.expected_task_ends = 3;
}
