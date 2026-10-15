//! The Metrics tab at its edges: a pod with more containers than a chart has lines, and two
//! pods of a workload that change places from one poll to the next.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use fresnel_core::app::ClusterRow;
use fresnel_core::cluster::ClusterId;
use fresnel_core::kinds::Kind;
use fresnel_core::metrics::{Amount, Cover, Metrics, MetricsSettings, Sample, SeriesKey};
use fresnel_core::store::ObjectKey;
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::Pod;
use serde_json::{Value, json};

use super::tab::{metrics_tab, tooltip};
use super::tables::wide;
use super::{PERIOD, of_kind, poll_time, scene_pods};
use crate::data::{changed, held, live, scene};
use crate::fake::{Window, shows, view};

/// The pod with many containers, and their count: a chart has 11 lines at most.
const MANY: &str = "half-done";
const CONTAINERS: u64 = 15;
/// The two pods of the Deployment `web`.
const WEB: [&str; 2] = ["web-6888b778b6-75md7", "web-6888b778b6-p2twl"];

/// The pods of the scene, with a change of the pod of this name.
fn pods_with(name: &str, change: impl Fn(&mut Value)) -> Vec<Arc<Pod>> {
    let one = |pod: Arc<Pod>| if pod.metadata.name.as_deref() == Some(name) { changed(&pod, &change) } else { pod };
    scene_pods().into_iter().map(one).collect()
}

/// Three polls of 15 s up to 5 s before now, each with these CPU values in millicores.
fn three_polls(values: &[(SeriesKey, u64)]) -> Metrics {
    let mut metrics = Metrics::waiting(ClusterId::new("ams"), PERIOD, poll_time(116));
    for poll in 117..120 {
        let at = poll_time(poll).duration_since(UNIX_EPOCH).expect("a time").as_secs();
        let at = u32::try_from(at).expect("a time of u32");
        let sample = |millicores: u64| Sample {
            at,
            cover: Cover::ONE,
            usage: Amount { cpu: millicores * 1_000_000, memory: 1 << 20 },
        };
        let samples: Vec<(SeriesKey, Sample)> =
            values.iter().map(|(key, millicores)| (key.clone(), sample(*millicores))).collect();
        metrics.take_samples(&samples, BTreeMap::new(), poll_time(poll), &MetricsSettings::default());
    }
    metrics
}

/// The names of the lines and marks of the tooltip of the CPU chart near now.
fn names(window: &mut Window) -> Vec<String> {
    let lines = tooltip(window, "CPU chart", 0.98);
    lines.iter().skip(1).map(|line| line.split(':').next().unwrap_or_default().to_owned()).collect()
}

/// A pod with 15 containers, of which `cN` uses (N + 1) * 10 millicores: the line of the pod
/// and the 10 largest containers in the sequence of their names, and a note for the 5
/// others. A line for each container would cost each frame its time (`MOST_LINES`).
#[test]
fn a_pod_with_more_containers_than_a_chart_has_lines_shows_the_largest_and_tells_the_others() {
    let containers: Vec<Value> = (0..CONTAINERS).map(|n| json!({"name": format!("c{n:02}"), "image": "app"})).collect();
    let pods = pods_with(MANY, |value| value["spec"]["containers"] = json!(containers));
    let pod = ObjectKey::new("scene", MANY);
    let mut values: Vec<(SeriesKey, u64)> = (0..CONTAINERS)
        .map(|n| (SeriesKey::Container { pod: pod.clone(), name: format!("c{n:02}").into() }, (n + 1) * 10))
        .collect();
    values.push((SeriesKey::Pod(pod), values.iter().map(|(_, millicores)| millicores).sum()));
    let row = live("ams", &pods);
    let mut data = row.data.clone().expect("the data of a connected cluster");
    data.metrics = held(three_polls(&values));
    let mut window = wide(vec![ClusterRow { data: Some(data), ..row }]);
    metrics_tab(&mut window, "Pods", MANY);
    let largest: Vec<String> = (5..CONTAINERS).map(|n| format!("c{n:02}")).collect();
    assert_eq!(names(&mut window), [vec!["pod".to_owned()], largest].concat());
    assert!(shows(&window, "5 more containers in the line \"pod\"."));
}

/// The two pods of `web` change places from one poll to the next. The color of a line is of
/// its place, so the lines keep their sequence, and each one its color.
#[test]
fn the_lines_keep_their_sequence_when_two_pods_change_places() {
    let web = |first: u64, second: u64| {
        let pod = |name: &str| SeriesKey::Pod(ObjectKey::new("scene", name));
        let workload = SeriesKey::Workload(Kind::Deployment, ObjectKey::new("scene", "web"));
        let metrics = three_polls(&[(pod(WEB[0]), first), (pod(WEB[1]), second), (workload, first + second)]);
        of_kind("ams", &scene::<Deployment>(), metrics)
    };
    let mut window = wide(vec![web(100, 200)]);
    metrics_tab(&mut window, "Deployments", "web");
    let before = names(&mut window);
    assert_eq!(before, ["all pods", WEB[0], WEB[1], "request", "limit"]);
    window.state_mut().view = view(vec![web(200, 100)]);
    window.run();
    assert_eq!(names(&mut window), before);
}
