//! The log views in the core (plan section 12.7): a view runs while the UI shows it, its
//! stream goes to the cluster of its pod only (weakness 9 of plan section 6), a view of a
//! cluster that connects later starts then, and a stream that ends starts again from the
//! second of its latest line with a marker and no line two times. The saves of a view go
//! the way of the commands of the UI, whose view tests have a fake core only.
//!
//! The store watches of a cluster are streams of the fake server too, so the test waits for
//! a state of the view, never for a count of open streams alone: the source is streaming
//! only after the server holds its stream, and its marker comes after a new stream opened.

use std::time::Duration;

use fresnel_core::app::{ClusterData, Command, CoreSettings, CoreView, Health};
use fresnel_core::cluster::ClusterId;
use fresnel_core::kinds::Kind;
use fresnel_core::logs::{
    LineKind, LogOptions, LogRequest, LogSave, LogView, SavePart, SaveState, SaveView, SourceState,
};
use fresnel_core::metrics::MetricsSettings;
use fresnel_core::store::ObjectKey;
use fresnel_testkit::{FakeApiServer, Fault, Home, SeenRequest};

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

/// The core with the clusters ams and fra, each with the pod `web-1`, once each store of ams
/// holds its watch. Only ams is selected.
struct Bed {
    app: App,
    ams: FakeApiServer,
    fra: FakeApiServer,
    /// The count of the watches of the stores of ams.
    watches: u64,
    home: Home,
}

impl Bed {
    async fn start(label: &str) -> Self {
        let (ams, fra) =
            (FakeApiServer::start().await.expect("a server"), FakeApiServer::start().await.expect("a server"));
        ams.set_body(PODS, &pods());
        fra.set_body(PODS, &pods());
        let home = home(label);
        write_kubeconfig(&home, "ams", &[("ams", ams.addr(), &ams, false), ("fra", fra.addr(), &fra, false)]);
        // The sampler of the metrics wakes the UI after each poll. With one poll in an hour,
        // only the new value of a log view wakes a wait of a test.
        let metrics = MetricsSettings { period: Duration::from_secs(3600), ..MetricsSettings::default() };
        let app = App::started(&home, CoreSettings { metrics, ..CoreSettings::default() });
        app.until_health("ams", Health::Live).await;
        let shown = app.until("each store of ams", |view| live_stores(view, "ams").is_some()).await;
        let watches = live_stores(&shown, "ams").unwrap_or_default();
        in_time("each watch of ams", ams.wait_for(|counters| counters.open_watches == watches)).await;
        Self { app, ams, fra, watches, home }
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
    let Bed { app, ams, fra, watches, home: _home } = Bed::start("logs").await;

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

/// Fast input: the UI shows and hides a view many times in a row. Each session that drops
/// ends its stream, and the last view has the one log stream of ams.
#[tokio::test(flavor = "multi_thread")]
async fn a_view_that_the_ui_shows_and_hides_fast_leaves_one_stream() {
    let bed = Bed::start("logs-fast").await;
    for _ in 0..50 {
        bed.app.core.send(Command::ShowLogs(vec![request("ams")]));
        bed.app.core.send(Command::ShowLogs(Vec::new()));
    }
    let mut last = request("ams");
    last.options.tail = Some(10);
    bed.app.core.send(Command::ShowLogs(vec![last.clone()]));
    let is_last =
        |view: &CoreView| streams(view, "ams") && log_view(view, "ams").is_some_and(|log| log.request == last);
    bed.app.until("the stream of the last view", is_last).await;
    let one_more = bed.watches + 1;
    in_time("one log stream", bed.ams.wait_for(|counters| counters.open_watches == one_more)).await;
    assert!(bed.ams.send_event(LOG, "2026-10-12T20:00:00.100000000Z hello") > 0);
    let shown = bed.app.until("the line", |view| log_view(view, "ams").is_some_and(|log| log.lines.len() == 1)).await;
    assert_eq!(data(&shown, "ams").map(|data| data.logs.len()), Some(1));
    assert!(bed.fra.requests().is_empty(), "fra is not selected");
}

/// The saves of the log views of a cluster, the oldest first.
fn saves(view: &CoreView, cluster: &str) -> Vec<SaveView> {
    data(view, cluster).map(|data| data.saves.iter().map(|save| SaveView::clone(save)).collect()).unwrap_or_default()
}

/// The commands `SaveLog` and `CancelSave` through the core: the lines of a view in a new
/// file of the folder of downloads, a full log whose server does not answer and its cancel,
/// which removes the file, and a save of a view that the UI does not show.
#[tokio::test(flavor = "multi_thread")]
async fn a_save_writes_the_lines_of_a_view_and_a_cancel_removes_the_file_of_a_full_log() {
    let mut bed = Bed::start("log-saves").await;
    bed.app.core.send(Command::ShowLogs(vec![request("ams")]));
    bed.app.until("the stream of ams", |view| streams(view, "ams")).await;
    bed.ams.send_event(LOG, "2026-10-12T20:00:00.100000000Z hello");
    bed.ams.send_event(LOG, "2026-10-12T20:00:00.200000000Z world");
    bed.app.until("two lines", |view| log_view(view, "ams").is_some_and(|log| log.lines.len() == 2)).await;
    let downloads = bed.home.path("Downloads");
    let files =
        || std::fs::read_dir(&downloads).map(|entries| entries.flatten().map(|entry| entry.path()).collect::<Vec<_>>());

    let save = |part, request| Command::SaveLog(LogSave { request, part, times: true });
    bed.app.core.send(save(SavePart::Buffer, request("ams")));
    let ended = |number: u64| {
        move |view: &CoreView| saves(view, "ams").iter().any(|save| save.number == number && save.state.is_final())
    };
    let shown = bed.app.until("the end of the first save", ended(1)).await;
    let first = saves(&shown, "ams").remove(0);
    let text = "2026-10-12T20:00:00.100000000Z hello\n2026-10-12T20:00:00.200000000Z world\n";
    assert_eq!(first.state, SaveState::Done { bytes: text.len() as u64, skipped: Vec::new() });
    let file = first.file.expect("the file of the save");
    assert_eq!(
        (file.parent(), std::fs::read_to_string(&file).ok().as_deref()),
        (Some(downloads.as_path()), Some(text))
    );

    // The server holds the request of the full log: the save waits, and a cancel ends it.
    bed.ams.add_fault(Fault::no_answer().on_path(LOG));
    bed.app.core.send(save(SavePart::Full, request("ams")));
    in_time("the request of the full log", bed.ams.wait_for(|counters| counters.unanswered == 1)).await;
    assert_eq!(log_requests(&bed.ams).last().map(query), Some("&container=app&timestamps=true"));
    assert_eq!(files().map(|files| files.len()).ok(), Some(2), "the file of the full log");
    bed.app.core.send(Command::CancelSave { cluster: ClusterId::new("ams"), number: 2 });
    let shown = bed.app.until("the end of the second save", ended(2)).await;
    assert_eq!(saves(&shown, "ams").get(1).map(|save| save.state.clone()), Some(SaveState::Cancelled));
    assert_eq!(files().ok(), Some(vec![file]), "the part of the full log is gone");

    // The core saves only a view that runs.
    let hidden = LogRequest { options: LogOptions { tail: Some(5), ..LogOptions::default() }, ..request("ams") };
    bed.app.core.send(save(SavePart::Buffer, hidden));
    let shown = bed.app.until("the refusal", ended(3)).await;
    let refused = saves(&shown, "ams").get(2).map(|save| save.state.clone());
    assert!(matches!(&refused, Some(SaveState::Failed(error)) if error.to_string() == "the log view is not open"));
    bed.app.expected_task_ends = 2;
}
