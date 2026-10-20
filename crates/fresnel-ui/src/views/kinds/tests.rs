//! The model of the table of a kind on the objects of the k3s fixture.

use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::time::SystemTime;

use fresnel_core::cluster::ClusterId;
use fresnel_core::index::{Index, IndexState, ObjectEvents};
use fresnel_core::kinds::Witness;
use fresnel_core::search::Source;
use fresnel_core::store::StoreState;
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::{ConfigMap, Namespace, Node, Pod};
use serde_json::json;

use super::*;
use crate::format::Clock;
use crate::testing::{TestTasks, fixture, fixture_pods, snapshot};
use crate::theme::DARK;
use crate::views::metrics::Problems;
use crate::views::objects::ClusterMark;
use crate::widgets::table::model::Lead;
use crate::widgets::tone::Tone;

fn titles<K: Describe>() -> Vec<&'static str> {
    KindModel::<K>::new().columns().iter().map(|column| column.title).collect()
}

/// The rows of the fixture objects of a kind in the clusters `ams` and `fra`.
fn rows<K: Describe>(bed: &TestTasks, filter: &ObjectFilter) -> Computed<KindRow<K>, Namespaces> {
    let objects = fixture::<K>();
    let clusters = ["ams", "fra"].map(|name| (ClusterId::new(name), bed.held(snapshot(&objects, StoreState::Live))));
    let never = AtomicU64::new(0);
    let data = Stores { clusters: clusters.to_vec(), metered: Vec::new(), witnesses: Vec::new() };
    KindModel::<K>::new().rows(&data, filter, &Cancel::new(&never, 0)).expect("no cancel")
}

fn names<K>(rows: &[KindRow<K>]) -> Vec<String> {
    rows.iter().map(|row| format!("{}/{}", row.id.cluster, row.id.key.name)).collect()
}

#[test]
fn the_cluster_column_follows_the_name_and_the_namespace_and_the_labels_wait_for_a_choice() {
    let deployments = titles::<Deployment>();
    assert_eq!(deployments[..4], ["Name", "Namespace", "Cluster", "Ready"]);
    assert_eq!(deployments[deployments.len() - 3..], ["Age", "CPU", "Memory"], "the metrics after the age");
    assert_eq!(titles::<Node>()[..3], ["Name", "Cluster", "Status"], "a kind with no namespace");
    assert!(!titles::<ConfigMap>().contains(&"CPU"), "a kind with no metrics");
    let model = KindModel::<Deployment>::new();
    let shown = |title: &str| model.columns().iter().find(|column| column.title == title).map(|column| column.shown);
    assert_eq!((shown("Labels"), shown("Images"), shown("Desired")), (Some(false), Some(true), Some(false)));
    assert_eq!(model.nouns().count(1), "1 Deployment");
    assert_eq!(model.nouns().count(2), "2 Deployments");
}

/// Weakness 1 of plan section 6: the rows are the objects of each cluster, each with the
/// identity of its cluster.
#[test]
fn the_rows_have_the_objects_of_each_cluster_and_the_filters_select_them() {
    let bed = TestTasks::new();
    let all = rows::<Deployment>(&bed, &ObjectFilter::default());
    assert_eq!((all.rows.len(), all.total), (8, 8));
    assert_eq!(all.summary.names, [Arc::from("kube-system"), Arc::from("scene")]);

    // A text of a column selects too: an image of the pods.
    let filter = |text: &str| ObjectFilter { text: text.to_owned(), ..ObjectFilter::default() };
    let mut pause = names(&rows::<Deployment>(&bed, &filter("MIRRORED-PAUSE")).rows);
    pause.sort();
    assert_eq!(pause, ["ams/web", "fra/web"]);
    assert_eq!(rows::<Deployment>(&bed, &filter("fra")).rows.len(), 4, "the cluster is a text of a row");

    let problems = ObjectFilter { problems: true, ..ObjectFilter::default() };
    let mut broken = names(&rows::<Deployment>(&bed, &problems).rows);
    broken.sort();
    assert_eq!(broken, ["ams/broken", "fra/broken"], "the Deployment with no ready pod");
    assert!(rows::<Namespace>(&bed, &ObjectFilter::default()).summary.names.is_empty(), "no namespace filter");
}

#[test]
fn a_sort_compares_the_values_and_the_last_key_gives_one_sequence() {
    let bed = TestTasks::new();
    let all = rows::<Deployment>(&bed, &ObjectFilter::default()).rows;
    let model = KindModel::<Deployment>::new();
    let find = |cluster: &str, name: &str| {
        all.iter().find(|row| row.id.cluster.as_str() == cluster && &*row.id.key.name == name).expect("a row")
    };
    let (web, broken) = (find("ams", "web"), find("ams", "broken"));
    let ready = model.columns().iter().position(|column| column.title == "Ready").expect("Ready");
    assert_eq!(model.compare(broken, web, ready), Ordering::Less, "0 of 1 ready before 2 of 2");
    assert_eq!(model.compare(web, find("fra", "web"), ready), Ordering::Equal);
    assert_eq!(model.tie(web, find("fra", "web")), Ordering::Less, "the same object in two clusters");
    assert_eq!(model.compare(web, broken, 99), Ordering::Equal, "a column that the model does not have");
}

/// Design rule 4: a row from a store that is not live has its mark, and no color of it
/// tells a state of now.
#[test]
fn the_cells_show_each_value_and_an_old_row_has_its_mark_and_no_state_color() {
    let bed = TestTasks::new();
    let all = rows::<Deployment>(&bed, &ObjectFilter::default()).rows;
    let marks = [
        (ClusterId::new("ams"), ClusterMark { color: DARK.clusters[0], old: None, problems: Problems::default() }),
        (
            ClusterId::new("fra"),
            ClusterMark {
                color: DARK.clusters[1],
                old: Some("old since 07:59:12".to_owned()),
                problems: Problems::default(),
            },
        ),
    ];
    let scene = RowScene { palette: &DARK, clock: Clock::fixed(SystemTime::now()), marks: HashMap::from(marks) };
    let model = KindModel::<Deployment>::new();
    let row = |cluster: &str| {
        all.iter().find(|row| row.id.cluster.as_str() == cluster && &*row.id.key.name == "broken").expect("a row")
    };
    let cells =
        |cluster: &str| -> Vec<Cell<'_>> { (0..5).map(|column| model.cell(row(cluster), column, &scene)).collect() };
    let live = cells("ams");
    let texts: Vec<&str> = live.iter().map(|cell| &*cell.text).collect();
    assert_eq!(texts, ["broken", "scene", "ams", "0/1", "1"]);
    assert_eq!(live[0].lead, Lead::Dot(DARK.error), "an object that is not healthy has a dot");
    assert_eq!((live[1].tone, live[2].lead), (Tone::Muted, Lead::Dot(DARK.clusters[0])));
    let old = cells("fra");
    assert_eq!(old[0].lead, Lead::Old("old since 07:59:12"));
    assert!(old.iter().all(|cell| cell.tone == Tone::Faint), "{old:?}");
    let healthy = all.iter().find(|row| &*row.id.key.name == "web").expect("a row");
    assert_eq!(model.cell(healthy, 0, &scene).lead, Lead::None, "a healthy object has no dot");
}

/// The user could not tell if a warning was still a problem, nor what a state means: the
/// table of the Events has the state of each warning now and its reason after its type, from
/// the live pods of its cluster.
#[test]
fn the_events_have_the_state_of_each_warning_now_after_their_type() {
    let bed = TestTasks::new();
    assert_eq!(titles::<Event>()[2..6], ["Cluster", "Type", "Now", "Reason"]);
    assert!(!titles::<Node>().contains(&"Now"), "an other kind");
    let ams = ClusterId::new("ams");
    let witness =
        Witness::new([Source::full(bed.held(snapshot(&fixture_pods(), StoreState::Live)))], SystemTime::now());
    let data = Stores {
        clusters: vec![(ams.clone(), bed.held(snapshot(&fixture::<Event>(), StoreState::Live)))],
        metered: Vec::new(),
        witnesses: vec![(ams.clone(), witness)],
    };
    let model = KindModel::<Event>::new();
    let never = AtomicU64::new(0);
    let rows = |filter: &ObjectFilter| model.rows(&data, filter, &Cancel::new(&never, 0)).expect("no cancel").rows;
    let all = rows(&ObjectFilter::default());
    let row = |name: &str| all.iter().find(|row| &*row.id.key.name == name).expect("a row");
    let (crash, normal) = (row("crash.18de28919fa8e1d0"), row("db-0.18de288fc7821e98"));
    let now = model.columns().iter().position(|column| column.title == "Now").expect("the column");
    let mark = ClusterMark { color: DARK.clusters[0], old: None, problems: Problems::default() };
    let scene =
        RowScene { palette: &DARK, clock: Clock::fixed(SystemTime::now()), marks: HashMap::from([(ams, mark)]) };
    let cell = model.cell(crash, now, &scene);
    let ongoing = "ongoing: the container crash waits: CrashLoopBackOff";
    assert_eq!((&*cell.text, cell.tone), (ongoing, Tone::Warn), "the BackOff of the pod crash");
    assert_eq!(cell.hint.as_deref(), Some("Ongoing now: the container crash waits: CrashLoopBackOff."));
    let no_store = model.cell(row("fails.18de2891a64a7373"), now, &scene).hint.map(String::from);
    assert_eq!(no_store.as_deref(), Some("Unknown: the cluster has no store of the Jobs."));
    assert_eq!(&*model.cell(normal, now, &scene).text, "–", "an Event that tells no problem");
    assert_eq!(model.compare(normal, crash, now), Ordering::Less, "no state first, as no value");
    let (unknown, resolved) = (row("fails.18de2891a64a7373"), row("crash.18de288e97918207"));
    assert_eq!(model.compare(unknown, resolved, now), Ordering::Less, "an unknown one can still be a problem");

    let found = |text: &str| rows(&ObjectFilter { text: text.to_owned(), ..ObjectFilter::default() });
    assert_eq!(found("ongoing").len(), 8, "the text filter finds a state");
    let of_old_pod =
        found("deleted").into_iter().any(|row| &*row.id.key.name == "web-64cb9bd5d4-c8kcm.18de288e9b577fa6");
    assert!(of_old_pod, "a pod of the old ReplicaSet of web");
}

/// An object of the fixture after a change of its JSON.
fn changed<T: serde::Serialize + serde::de::DeserializeOwned>(
    object: &T,
    change: impl FnOnce(&mut serde_json::Value),
) -> Arc<T> {
    let mut value = serde_json::to_value(object).expect("JSON");
    change(&mut value);
    Arc::new(serde_json::from_value(value).expect("an object"))
}

/// The user asked what "unknown" means: the Now cell of a quiet warning has its time with no
/// report and its reason, and the quiet ones sort after the unknown ones, before the ended.
#[test]
fn a_quiet_warning_has_its_time_and_its_reason_in_the_now_cell() {
    let bed = TestTasks::new();
    const COREDNS: &str = "coredns-64fd4b4794-6lsjp";
    let started = json!({"running": {"startedAt": "2026-10-13T17:00:00Z"}});
    let pods: Vec<Arc<Pod>> = fixture_pods()
        .iter()
        .map(|pod| match pod.metadata.name.as_deref() {
            Some(COREDNS) => changed(&**pod, |pod| pod["status"]["containerStatuses"][0]["state"] = started.clone()),
            _ => Arc::clone(pod),
        })
        .collect();
    // The probe takes the place of the failed sandbox: the kubelet reported nothing newer.
    let events: Vec<Arc<Event>> = fixture::<Event>()
        .iter()
        .map(|event| match (event.involved_object.name.as_deref(), event.reason.as_deref()) {
            (Some(COREDNS), Some("FailedCreatePodSandBox")) => changed(&**event, |event| {
                (event["reason"], event["count"]) = ("Unhealthy".into(), 4.into());
                event["message"] = "Liveness probe failed: HTTP probe failed with statuscode: 503".into();
                event["involvedObject"]["fieldPath"] = "spec.containers{coredns}".into();
                (event["firstTimestamp"], event["lastTimestamp"]) =
                    ("2026-10-13T17:23:06Z".into(), "2026-10-13T17:24:36Z".into());
            }),
            _ => Arc::clone(event),
        })
        .collect();
    let at: SystemTime = "2026-10-13T18:00:00Z".parse::<k8s_openapi::jiff::Timestamp>().expect("a time").into();
    let events = snapshot(&events, StoreState::Live);
    let index = bed.held(Index::<ObjectEvents>::of_snapshot(&events, IndexState::Following));
    let witness = Witness::new([Source::full(bed.held(snapshot(&pods, StoreState::Live)))], at)
        .with_reports(index, &events.status(Kind::Event));
    let ams = ClusterId::new("ams");
    let data = Stores {
        clusters: vec![(ams.clone(), bed.held(events))],
        metered: Vec::new(),
        witnesses: vec![(ams.clone(), witness)],
    };
    let model = KindModel::<Event>::new();
    let never = AtomicU64::new(0);
    let rows = |filter: &ObjectFilter| model.rows(&data, filter, &Cancel::new(&never, 0)).expect("no cancel").rows;
    let all = rows(&ObjectFilter::default());
    let row = |name: &str| all.iter().find(|row| &*row.id.key.name == name).expect("a row");
    let quiet = row("coredns-64fd4b4794-6lsjp.18de288e9d37f6d6");
    let now = model.columns().iter().position(|column| column.title == "Now").expect("the column");
    let mark = ClusterMark { color: DARK.clusters[0], old: None, problems: Problems::default() };
    let scene = RowScene { palette: &DARK, clock: Clock::fixed(at), marks: HashMap::from([(ams, mark)]) };
    let cell = model.cell(quiet, now, &scene);
    let text = "quiet 35 min: the kubelet reports a problem that goes on again at least each 8 min, and the \
                container coredns runs since before the last report";
    assert_eq!((&*cell.text, cell.tone), (text, Tone::Muted));
    let (unknown, resolved) = (row("fails.18de2891a64a7373"), row("crash.18de288e97918207"));
    assert_eq!(model.compare(unknown, quiet, now), Ordering::Less, "an unknown one can still be a problem");
    assert_eq!(model.compare(quiet, resolved, now), Ordering::Less, "a quiet one did not end with a proof");
    let found = rows(&ObjectFilter { text: "quiet".to_owned(), ..ObjectFilter::default() });
    assert_eq!(names(&found), ["ams/coredns-64fd4b4794-6lsjp.18de288e9d37f6d6"], "the text filter finds the state");
}
