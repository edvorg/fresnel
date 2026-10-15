//! The log views in the core (plan section 12.7): a view runs while the UI shows it, its
//! stream goes to the cluster of its pod only (weakness 9 of plan section 6), a view of a
//! cluster that connects later starts then, and a stream that ends starts again from the
//! second of its latest line with a marker and no line two times.
//!
//! The store watches of a cluster are streams of the fake server too, so the test waits for
//! a state of the view, never for a count of open streams alone: the source is streaming
//! only after the server holds its stream, and its marker comes after a new stream opened.

use std::time::Duration;

use fresnel_core::app::{ClusterData, Command, CoreSettings, CoreView, Health};
use fresnel_core::cluster::ClusterId;
use fresnel_core::kinds::Kind;
use fresnel_core::logs::{LineKind, LogOptions, LogRequest, LogView, SourceState};
use fresnel_core::metrics::MetricsSettings;
use fresnel_core::store::ObjectKey;
use fresnel_testkit::{FakeApiServer, SeenRequest};

use super::{App, home, row, write_kubeconfig};
use crate::common::in_time;

const PODS: &str = "/api/v1/pods";
const LOG: &str = "/api/v1/namespaces/shop/pods/web-1/log";

/// The pod `web-1` with its container `app`, which runs.
fn pods() -> String {
    let pod = r#"{"metadata":{"name":"web-1","namespace":"shop","uid":"u1","resourceVersion":"9"},
        "spec":{"containers":[{"name":"app"}]},
        "status":{"phase":"Running","containerStatuses":[{"name":"app","image":"a","imageID":"","ready":true,
        "restartCount":0,"containerID":"containerd://a1","state":{"running":{}}}]}}"#;
    format!(r#"{{"kind":"PodList","apiVersion":"v1","metadata":{{"resourceVersion":"10"}},"items":[{pod}]}}"#)
}

fn request(cluster: &str) -> LogRequest {
    LogRequest {
        cluster: ClusterId::new(cluster),
        kind: Kind::Pod,
        key: ObjectKey::new("shop", "web-1"),
        uid: "u1".into(),
        options: LogOptions::default(),
    }
}

fn data<'a>(view: &'a CoreView, cluster: &str) -> Option<&'a ClusterData> {
    row(view, cluster).data.as_ref()
}

/// The log view of a cluster in the view of the core.
fn log_view(view: &CoreView, cluster: &str) -> Option<LogView> {
    data(view, cluster)?.logs.first().map(|held| LogView::clone(held))
}

/// True when the source of the log view of the cluster streams: the server holds its stream.
fn streams(view: &CoreView, cluster: &str) -> bool {
    log_view(view, cluster).is_some_and(|log| log.sources.first().is_some_and(|s| s.state == SourceState::Streaming))
}

/// The count of the stores of a cluster when each one is live: each one holds a watch.
fn live_stores(view: &CoreView, cluster: &str) -> Option<u64> {
    let data = data(view, cluster)?;
    let live = data.pods.items.len() == 1 && data.stores.iter().all(|store| store.state.is_live());
    live.then(|| data.stores.len() as u64)
}

/// The texts of the lines of the view, with a mark for a marker.
fn texts(view: &LogView) -> Vec<String> {
    let lines = &view.lines;
    let text = |line: &fresnel_core::logs::LogLine| match line.kind {
        LineKind::Log => line.text.to_string(),
        LineKind::Marker => format!("marker: {}", line.text),
    };
    lines.range(lines.first(), lines.end()).map(|(_, line)| text(line)).collect()
}

fn log_requests(server: &FakeApiServer) -> Vec<SeenRequest> {
    server.requests().into_iter().filter(|seen| seen.uri.path() == LOG).collect()
}

fn query(seen: &SeenRequest) -> &str {
    seen.uri.query().unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_log_view_follows_its_pod_in_its_cluster_and_a_new_stream_loses_no_line() {
    let (ams, fra) = (FakeApiServer::start().await.expect("a server"), FakeApiServer::start().await.expect("a server"));
    ams.set_body(PODS, &pods());
    fra.set_body(PODS, &pods());
    let home = home("logs");
    write_kubeconfig(&home, "ams", &[("ams", ams.addr(), &ams, false), ("fra", fra.addr(), &fra, false)]);
    // The sampler of the metrics wakes the UI after each poll. With one poll in an hour, only
    // the new value of a log view wakes a wait of this test.
    let metrics = MetricsSettings { period: Duration::from_secs(3600), ..MetricsSettings::default() };
    let app = App::started(&home, CoreSettings { metrics, ..CoreSettings::default() });
    app.until_health("ams", Health::Live).await;
    let shown = app.until("each store of ams", |view| live_stores(view, "ams").is_some()).await;
    let watches = live_stores(&shown, "ams").unwrap_or_default();
    in_time("each watch of ams", ams.wait_for(|counters| counters.open_watches == watches)).await;

    app.core.send(Command::ShowLogs(vec![request("ams")]));
    app.until("the stream of ams", |view| streams(view, "ams")).await;
    let first = log_requests(&ams);
    assert_eq!(
        first.iter().map(query).collect::<Vec<_>>(),
        ["&container=app&follow=true&tailLines=1000&timestamps=true"]
    );
    assert!(ams.send_event(LOG, "2026-10-12T20:00:00.100000000Z hello") > 0);
    app.until("the first line", |view| log_view(view, "ams").is_some_and(|log| log.lines.len() == 1)).await;

    // The server ends each stream, and the container still runs: the next stream starts at
    // the second of the latest line and sends that line again. Its marker comes after its open.
    assert!(ams.end_watches() > 0);
    app.until("the marker of the new stream", |view| log_view(view, "ams").is_some_and(|log| log.lines.len() == 2))
        .await;
    let again = log_requests(&ams);
    assert_eq!(again.len(), 2);
    assert_eq!(query(&again[1]), "&container=app&follow=true&sinceTime=2026-10-12T20%3A00%3A00Z&timestamps=true");
    ams.send_event(LOG, "2026-10-12T20:00:00.100000000Z hello");
    ams.send_event(LOG, "2026-10-12T20:00:00.200000000Z world");
    let shown = app.until("the new line", |view| log_view(view, "ams").is_some_and(|log| log.lines.len() == 3)).await;
    let log = log_view(&shown, "ams").expect("the view");
    let marker = "marker: the stream started again after: the stream ended while the container runs";
    assert_eq!(texts(&log), ["hello", marker, "world"]);
    assert_eq!((log.sources[0].lines, log.sources[0].repeated), (2, 1));

    // A view of fra, which is not connected yet, starts when fra connects. The view of ams
    // stays with its session and its lines.
    app.core.send(Command::ShowLogs(vec![request("ams"), request("fra")]));
    app.select("fra", true);
    let shown = app.until("the stream of fra", |view| streams(view, "fra")).await;
    assert_eq!(log_requests(&fra).len(), 1, "the view of fra asks fra");
    assert_eq!((log_requests(&ams).len(), log_view(&shown, "ams").map(|log| log.lines.len())), (2, Some(3)));

    // The views close: their streams end, and no cluster has a view.
    app.core.send(Command::ShowLogs(Vec::new()));
    let closed =
        |view: &CoreView| ["ams", "fra"].iter().all(|cluster| data(view, cluster).is_some_and(|d| d.logs.is_empty()));
    app.until("no view", closed).await;
    in_time("the stream of ams closes", ams.wait_for(|counters| counters.open_watches == watches)).await;
    let shown = app.until("each store of fra", |view| live_stores(view, "fra").is_some()).await;
    let fra_watches = live_stores(&shown, "fra").unwrap_or_default();
    in_time("the stream of fra closes", fra.wait_for(|counters| counters.open_watches == fra_watches)).await;
    assert_eq!((log_requests(&ams).len(), log_requests(&fra).len()), (2, 1), "no new stream");
}
