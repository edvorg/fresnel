//! The log views in the core (plan section 12.7): a view runs while the UI shows it, its
//! stream goes to the cluster of its pod only (weakness 9 of plan section 6), and a stream
//! that ends starts again from the second of its latest line with a marker and no line two
//! times.

use fresnel_core::app::{Command, CoreSettings, CoreView, Health};
use fresnel_core::cluster::ClusterId;
use fresnel_core::kinds::Kind;
use fresnel_core::logs::{LineKind, LogOptions, LogRequest, LogView, SourceState};
use fresnel_core::store::ObjectKey;
use fresnel_testkit::{FakeApiServer, SeenRequest};

use super::{App, home, row, write_kubeconfig};

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

/// The log view of a cluster in the view of the core.
fn log_view(view: &CoreView, cluster: &str) -> Option<LogView> {
    row(view, cluster).data.as_ref()?.logs.first().map(|held| LogView::clone(held))
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

#[tokio::test(flavor = "multi_thread")]
async fn a_log_view_follows_its_pod_in_its_cluster_and_a_new_stream_loses_no_line() {
    let (ams, fra) = (FakeApiServer::start().await.expect("a server"), FakeApiServer::start().await.expect("a server"));
    ams.set_body(PODS, &pods());
    fra.set_body(PODS, &pods());
    let home = home("logs");
    write_kubeconfig(&home, "ams", &[("ams", ams.addr(), &ams, false), ("fra", fra.addr(), &fra, false)]);
    let app = App::started(&home, CoreSettings::default());
    app.select("fra", true);
    app.until_health("ams", Health::Live).await;
    let pod_loaded = |view: &CoreView| {
        row(view, "ams").data.as_ref().is_some_and(|data| data.pods.state.is_live() && data.pods.items.len() == 1)
    };
    app.until("the pod of ams", pod_loaded).await;

    let streams = ams.counters().open_watches;
    app.core.send(Command::ShowLogs(vec![request("ams")]));
    ams.wait_for(|counters| counters.open_watches > streams).await;
    let first = log_requests(&ams);
    assert_eq!(first.len(), 1);
    let query = first[0].uri.query().unwrap_or_default();
    assert_eq!(query, "&container=app&follow=true&tailLines=1000&timestamps=true");
    assert!(ams.send_event(LOG, "2026-10-12T20:00:00.100000000Z hello") > 0);
    let shown = app.until("the first line", |view| log_view(view, "ams").is_some_and(|log| log.lines.len() == 1)).await;
    let log = log_view(&shown, "ams").expect("the view");
    assert_eq!((texts(&log), log.sources[0].state.clone()), (vec!["hello".to_owned()], SourceState::Streaming));
    assert!(log_view(&shown, "fra").is_none() && log_requests(&fra).is_empty(), "the view is of ams only");

    // The server ends each stream, and the container still runs: the next stream starts at
    // the second of the latest line and sends that line again.
    let streams = ams.counters().open_watches;
    assert!(ams.end_watches() > 0);
    app.until("a second stream", |_| log_requests(&ams).len() == 2).await;
    ams.wait_for(|counters| counters.open_watches >= streams).await;
    let query = log_requests(&ams)[1].uri.query().unwrap_or_default().to_owned();
    assert_eq!(query, "&container=app&follow=true&sinceTime=2026-10-12T20%3A00%3A00Z&timestamps=true");
    ams.send_event(LOG, "2026-10-12T20:00:00.100000000Z hello");
    ams.send_event(LOG, "2026-10-12T20:00:00.200000000Z world");
    let shown = app.until("the new line", |view| log_view(view, "ams").is_some_and(|log| log.lines.len() == 3)).await;
    let log = log_view(&shown, "ams").expect("the view");
    let marker = "marker: the stream started again after: the stream ended while the container runs";
    assert_eq!(texts(&log), ["hello", marker, "world"]);
    assert_eq!((log.sources[0].lines, log.sources[0].repeated), (2, 1));

    // The view closes: its stream ends, and the cluster has no view.
    let streams = ams.counters().open_watches;
    app.core.send(Command::ShowLogs(Vec::new()));
    app.until("no view", |view| row(view, "ams").data.as_ref().is_some_and(|data| data.logs.is_empty())).await;
    ams.wait_for(|counters| counters.open_watches < streams).await;
    assert!(log_requests(&fra).is_empty(), "fra got no request of a log");
}
