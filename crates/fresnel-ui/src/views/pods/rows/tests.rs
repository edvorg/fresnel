//! The rows of the Pod table on the pods of the k3s fixture.

use std::sync::atomic::AtomicU64;

use fresnel_core::store::StoreState;

use super::*;
use crate::testing::{TestTasks, fixture_pods, snapshot};
use crate::theme::DARK;

/// The rows of the fixture pods in the clusters `ams` and `fra`, for this filter.
fn rows(bed: &TestTasks, filter: &PodFilter) -> Computed<PodRow, PodSummary> {
    let pods = fixture_pods();
    let clusters = ["ams", "fra"].map(|name| (ClusterId::new(name), bed.held(snapshot(&pods, StoreState::Live))));
    let never = AtomicU64::new(0);
    PodModel.rows(&PodData { clusters: clusters.to_vec() }, filter, &Cancel::new(&never, 0)).expect("no cancel")
}

fn names(rows: &[PodRow]) -> Vec<String> {
    let mut names: Vec<String> = rows.iter().map(|row| format!("{}/{}", row.id.cluster, row.key.name)).collect();
    names.sort();
    names
}

fn row<'a>(rows: &'a [PodRow], cluster: &str, name: &str) -> &'a PodRow {
    rows.iter().find(|row| row.id.cluster.as_str() == cluster && &*row.key.name == name).expect("the row")
}

/// Weakness 1 of plan section 6: the rows are the pods of each cluster, and a row
/// carries the identity of its cluster.
#[test]
fn each_filter_selects_pods_of_each_cluster_and_the_count_is_of_all_pods() {
    let bed = TestTasks::new();
    let all = rows(&bed, &PodFilter::default());
    assert_eq!((all.rows.len(), all.total), (50, 50));
    assert_eq!(all.summary.namespaces.iter().map(|name| &**name).collect::<Vec<_>>(), ["kube-system", "scene"]);
    let twice = row(&all.rows, "ams", "crash").id.uid == row(&all.rows, "fra", "crash").id.uid;
    assert!(twice && row(&all.rows, "ams", "crash").id != row(&all.rows, "fra", "crash").id, "one UID, two objects");

    let system = PodFilter { namespaces: [Arc::from("kube-system")].into(), ..PodFilter::default() };
    let system = rows(&bed, &system);
    assert_eq!((system.rows.len(), system.total), (4, 50), "the count before the filter stays");
    assert_eq!(system.summary.namespaces.len(), 2, "the menu lists each namespace, also with a filter");

    // The text finds a part of a name and a part of a status, with no difference of case.
    let crash = rows(&bed, &PodFilter { text: " CRASH ".to_owned(), ..PodFilter::default() });
    assert_eq!(names(&crash.rows), ["ams/crash", "ams/init-crash", "fra/crash", "fra/init-crash"]);
    let by_cluster = rows(&bed, &PodFilter { text: "fra".to_owned(), ..PodFilter::default() });
    assert_eq!(by_cluster.rows.len(), 25, "the text finds the cluster of a row");

    let problems = rows(&bed, &PodFilter { problems: true, ..PodFilter::default() });
    let of_ams: Vec<String> = names(&problems.rows).into_iter().filter(|name| name.starts_with("ams/")).collect();
    let expected = [
        "broken-6bf768c954-gjvsw",
        "crash",
        "failed",
        "fails-l8dvz",
        "half-done",
        "init-crash",
        "init-error",
        "no-config",
        "not-ready",
        "unschedulable",
    ];
    assert_eq!(of_ams, expected.map(|name| format!("ams/{name}")), "the pods that are not healthy");
    assert!(problems.rows.iter().all(|row| row.status.health.is_problem()));
}

#[test]
fn a_cancel_ends_the_computation_with_no_rows() {
    let bed = TestTasks::new();
    let data =
        PodData { clusters: vec![(ClusterId::new("ams"), bed.held(snapshot(&fixture_pods(), StoreState::Live)))] };
    let newest = AtomicU64::new(2);
    assert!(PodModel.rows(&data, &PodFilter::default(), &Cancel::new(&newest, 1)).is_none());
}

/// Plan section 11.2: each sort has a fixed last key, and a sort value has no clock.
#[test]
fn each_column_has_a_typed_sequence_and_two_pods_are_never_equal() {
    let bed = TestTasks::new();
    let all = rows(&bed, &PodFilter::default()).rows;
    let (crash, web, done) =
        (row(&all, "ams", "crash"), row(&all, "ams", "web-6888b778b6-p2twl"), row(&all, "ams", "done"));
    let column = |title: &str| COLUMNS.iter().position(|column| column.title == title).expect("the column");
    let compare = |a: &PodRow, b: &PodRow, title: &str| PodModel.compare(a, b, column(title));
    assert_eq!(compare(crash, web, "Name"), Ordering::Less);
    assert_eq!(compare(crash, web, "Namespace"), Ordering::Equal);
    assert_eq!(compare(crash, row(&all, "fra", "crash"), "Cluster"), Ordering::Less);
    assert_eq!(compare(crash, web, "Ready"), Ordering::Less, "0/1 is before 1/1");
    assert_eq!(compare(crash, web, "Status"), Ordering::Less, "CrashLoopBackOff is before Running");
    assert_eq!(compare(crash, web, "Restarts"), Ordering::Greater, "a number, not a text: 2 is after 0");
    assert_eq!(compare(done, crash, "Last restart"), Ordering::Less, "no restart comes first");
    assert_eq!(compare(row(&all, "ams", "gated"), crash, "Node"), Ordering::Less, "no node comes first");
    // The pod `web` is 14 s younger than the pod `crash`: a small age is first.
    assert_eq!(compare(web, crash, "Age"), Ordering::Less);
    assert_eq!(PodModel.compare(crash, web, COLUMNS.len()), Ordering::Equal, "a column that the table does not have");

    for a in &all {
        for b in &all {
            let same = std::ptr::eq(a, b);
            assert_eq!(PodModel.tie(a, b) == Ordering::Equal, same, "{:?} and {:?}", a.id, b.id);
        }
    }
    assert_eq!(PodModel.tie(crash, row(&all, "fra", "crash")), Ordering::Less, "the same pod in two clusters");
}

/// Design rule 4: a row from a store that is not live has its mark, and no color of it
/// tells a state of now.
#[test]
fn a_row_of_a_store_that_is_not_live_has_the_mark_and_no_state_color() {
    let bed = TestTasks::new();
    let all = rows(&bed, &PodFilter::default()).rows;
    let marks = [
        (ClusterId::new("ams"), ClusterMark { color: DARK.clusters[0], old: None }),
        (ClusterId::new("fra"), ClusterMark { color: DARK.clusters[1], old: Some("old since 07:59:12".to_owned()) }),
    ];
    let clock = Clock::fixed(SystemTime::UNIX_EPOCH);
    let mut scene = PodScene { palette: &DARK, clock, marks: marks.into() };
    let cells = |row: &PodRow, scene: &PodScene| -> Vec<(String, Tone, Lead<'static>)> {
        let cell = |column| PodModel.cell(row, column, scene);
        (0..COLUMNS.len())
            .map(|column| {
                let Cell { text, tone, lead } = cell(column);
                let lead = match lead {
                    Lead::Old(_) => Lead::Old("old"),
                    Lead::Dot(color) => Lead::Dot(color),
                    Lead::None => Lead::None,
                };
                (text.into_owned(), tone, lead)
            })
            .collect()
    };

    let live = cells(row(&all, "ams", "crash"), &scene);
    let texts: Vec<&str> = live.iter().map(|(text, _, _)| text.as_str()).collect();
    assert_eq!(texts[..6], ["crash", "scene", "ams", "0/1", "CrashLoopBackOff", "2"]);
    assert_eq!(live[0].2, Lead::None, "a row of a live store has no mark");
    assert_eq!((live[2].2, live[3].1), (Lead::Dot(DARK.clusters[0]), Tone::Warn));
    assert_eq!((live[4].1, live[4].2), (Tone::Error, Lead::Dot(DARK.error)));

    let old = cells(row(&all, "fra", "crash"), &scene);
    assert_eq!(old[0].2, Lead::Old("old"));
    assert!(old.iter().all(|(_, tone, _)| *tone == Tone::Faint), "{old:?}");
    let old_texts: Vec<&str> = old.iter().map(|(text, _, _)| text.as_str()).collect();
    assert_eq!((old_texts[2], &old_texts[3..]), ("fra", &texts[3..]), "the values stay (design rule 5)");

    // The cluster left the selection, and the rows of the next result do not have it.
    scene.marks.remove(&ClusterId::new("ams"));
    let left = PodModel.cell(row(&all, "ams", "crash"), 0, &scene);
    assert_eq!(left.lead, Lead::Old(NOT_SELECTED));
}
