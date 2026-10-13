//! The Pod status text of the app against the row of the API server and of kubectl.

use std::collections::BTreeMap;

use fresnel_core::kinds::{Assess, Full, ObjectHealth, PodStatus};
use fresnel_core::store::{ObjectKey, Snapshot};
use fresnel_testkit::PodLine;
use http::header::ACCEPT;
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::{Namespace, Pod, Service};
use serde_json::Value;

use super::read::text;
use super::scene::{self, ENDING, NAMESPACE};
use crate::bed::{App, Cluster};

/// The most reads for one moment in which the store, the API and kubectl show the same pods.
const ATTEMPTS: usize = 40;

/// What the Pod table shows of one pod: Ready, Status, Restarts, and whether the row has
/// the time of the last restart.
type Row = (String, String, u32, bool);

fn of_app(pod: &Pod) -> Row {
    let status = PodStatus::of(pod);
    (format!("{}/{}", status.ready, status.containers), status.text, status.restarts, status.last_restart.is_some())
}

fn of_kubectl(line: &PodLine) -> Row {
    (line.ready.clone(), line.status.clone(), line.restarts, line.restarted_ago)
}

/// The table of the pods as the API server prints it for kubectl: the cells of each row
/// with the resource version of its pod.
async fn table(cluster: &Cluster) -> BTreeMap<ObjectKey, (String, Row)> {
    let request = http::Request::get("/api/v1/pods?includeObject=Metadata")
        .header(ACCEPT, "application/json;as=Table;g=meta.k8s.io;v=v1")
        .body(Vec::new());
    let table: Value = cluster.api.gateway.list_page(request.expect("a request")).await.expect("the table of the pods");
    let row = |row: &Value| {
        let cell = |number: usize| text(row, &format!("/cells/{number}")).to_owned();
        // The cell is `3` or `3 (25s ago)`.
        let (restarts, ago) = (cell(3), cell(3).ends_with("ago)"));
        let count = restarts.split(' ').next().and_then(|count| count.parse().ok()).expect("a count of restarts");
        let key = ObjectKey::new(text(row, "/object/metadata/namespace"), text(row, "/object/metadata/name"));
        (key, (text(row, "/object/metadata/resourceVersion").to_owned(), (cell(1), cell(2), count, ago)))
    };
    table["rows"].as_array().expect("rows").iter().map(row).collect()
}

fn versions(pods: &Snapshot<Pod>) -> BTreeMap<ObjectKey, String> {
    let version = |pod: &Pod| pod.metadata.resource_version.clone().unwrap_or_default();
    pods.items.iter().map(|(key, pod)| (key.clone(), version(pod))).collect()
}

/// For each pod of the cluster, the app shows the row of `kubectl get pods`, and the row
/// that the API server prints for the same version of the pod.
pub(crate) async fn each_pod_has_the_row_of_kubectl_and_of_the_api(cluster: &Cluster, app: &App) {
    let pods = app.stores.stores.full::<Pod>().expect("the store of the pods");
    for attempt in 1..=ATTEMPTS {
        let before = pods.read();
        let lines = scene::pod_lines(cluster).await.1;
        let table = table(cluster).await;
        let of_table: BTreeMap<_, _> = table.iter().map(|(key, (version, _))| (key.clone(), version.clone())).collect();
        // No pod changed between the reads: the three sources show one moment.
        if pods.read().generation != before.generation || versions(&before) != of_table {
            continue;
        }
        assert_eq!(lines.len(), table.len(), "kubectl and the API show the same pods");
        for line in &lines {
            let key = ObjectKey::new(&line.namespace, &line.name);
            let row = of_app(before.items.get(&key).expect("the pod of the line is in the store"));
            assert_eq!(row, of_kubectl(line), "{}/{}: the app and kubectl", line.namespace, line.name);
            assert_eq!(row, table[&key].1, "{}/{}: the app and the API server", line.namespace, line.name);
            // Each pod of the scene is in the state of its line of the scene.
            let at_rest = scene::at_rest(line).expect("a pod of the scene or of k3s");
            assert_eq!((row.0.as_str(), row.1.as_str()), at_rest, "{}", line.name);
        }
        let restarted = lines.iter().filter(|line| line.restarted_ago).count();
        println!("{} pods: the app shows the row of kubectl and of the API (read {attempt})", lines.len());
        assert!(restarted >= 2, "the scene has two pods with a last restart, and {restarted} lines show one");
        return;
    }
    panic!("the pods changed in each of {ATTEMPTS} reads");
}

/// The health value of one object of the live scene, read from its store of the app.
fn health<K: Full>(app: &App, namespace: &str, name: &str) -> ObjectHealth {
    let objects = app.stores.stores.full::<K>().expect("the store of the kind").read();
    let object = objects.items.get(&ObjectKey::new(namespace, name));
    object.unwrap_or_else(|| panic!("the store has no {:?} {name}", K::KIND)).health()
}

/// One object for each value: the scene on the live cluster gives the values of the fixture.
pub(crate) fn the_objects_of_the_scene_have_their_health_value(app: &App) {
    assert_eq!(health::<Deployment>(app, NAMESPACE, "web"), ObjectHealth::Healthy);
    assert_eq!(health::<Deployment>(app, NAMESPACE, "broken"), ObjectHealth::Failed, "the rollout passed its deadline");
    assert_eq!(health::<Service>(app, NAMESPACE, "public"), ObjectHealth::Progressing, "no address yet");
    assert_eq!(health::<Namespace>(app, "", ENDING), ObjectHealth::Progressing, "its delete waits for a finalizer");
    assert_eq!(health::<Pod>(app, NAMESPACE, "not-ready"), ObjectHealth::Warning);
    let pods = app.stores.stores.full::<Pod>().expect("the store of the pods").read();
    let problems = pods.items.iter().filter(|(_, pod)| pod.health().is_problem()).count();
    assert_eq!(problems, 10, "the pods of the scene that the problems filter shows");
}
