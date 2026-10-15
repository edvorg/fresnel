//! The model of the table of a kind on the objects of the k3s fixture.

use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::time::SystemTime;

use fresnel_core::cluster::ClusterId;
use fresnel_core::store::StoreState;
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::{ConfigMap, Namespace, Node};

use super::*;
use crate::format::Clock;
use crate::testing::{TestTasks, fixture, snapshot};
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
    let data = Stores { clusters: clusters.to_vec(), metered: Vec::new() };
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
