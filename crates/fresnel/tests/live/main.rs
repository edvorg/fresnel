//! The views of the app on its real core, with no window (plan sections 6 and 15).
//!
//! The core reads the pods of two fake API servers through fault proxies, and the test
//! draws the frames of the window with [`fresnel::frame::draw`]. So the test covers the
//! way from a fault of the network to the freshness mark of a row. It uses real time, and
//! each wait ends at a frame that shows what the test waits for.

#![allow(clippy::expect_used, reason = "a test stops with a panic")]

#[path = "../common/mod.rs"]
mod common;
#[path = "../common/window.rs"]
mod window;

use std::time::{Duration, Instant};

use eframe::egui;
use egui_kittest::kittest::Queryable as _;
use fresnel_testkit::{FakeApiServer, FaultProxy, Home, ProxyMode};
use serde_json::Value;
use tokio::runtime::{Builder, Runtime};

use window::{LIMIT, Window, core, shows, stop_with_no_ended_task, until, window};

/// The path of the Pod list and of the Pod watch.
const PODS: &str = "/api/v1/pods";

/// One fake cluster: its server, and the proxy that the app connects to.
struct Cluster {
    proxy: FaultProxy,
    server: FakeApiServer,
}

/// The pods of the k3s fixture of `fresnel-core` as the body of a Pod list. `suffix`
/// stands after each name and each UID, so the two clusters have pods of their own, and
/// a sort by name mixes their rows.
fn pod_list(suffix: &str) -> String {
    let text = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../fresnel-core/fixtures/k3s/pods.json"));
    let mut pods: Vec<Value> = serde_json::from_str(text).expect("the fixture");
    for pod in &mut pods {
        for field in ["name", "uid"] {
            let value = pod["metadata"][field].as_str().expect("a text").to_owned();
            pod["metadata"][field] = format!("{value}{suffix}").into();
        }
    }
    serde_json::json!({"metadata": {"resourceVersion": "7"}, "items": pods}).to_string()
}

async fn cluster(suffix: &str) -> Cluster {
    let server = FakeApiServer::start().await.expect("a server");
    server.set_body(PODS, &pod_list(suffix));
    let proxy = FaultProxy::start(server.addr()).await.expect("a proxy");
    Cluster { proxy, server }
}

/// A home folder whose kubeconfig has the two clusters, with `ams` as the cluster of `kubectl`.
fn home(ams: &Cluster, fra: &Cluster) -> Home {
    let home = Home::new("live", &["kubeconfig.yaml"]).expect("a home folder");
    let entry = |name: &str, cluster: &Cluster| {
        let (address, authority) = (cluster.proxy.addr(), common::authority(&cluster.server));
        format!(
            "- {{name: {name}, cluster: {{server: 'https://{address}', certificate-authority-data: {authority}}}}}\n"
        )
    };
    let text = format!(
        "current-context: ams\nclusters:\n{}{}contexts:\n- {{name: ams, context: {{cluster: ams}}}}\n- {{name: fra, context: {{cluster: fra}}}}\n",
        entry("ams", ams),
        entry("fra", fra)
    );
    home.write("work/kubeconfig.yaml", &text).expect("the kubeconfig");
    home
}

/// The count of rows on the screen whose name ends with this text.
fn rows(window: &Window, suffix: &str) -> usize {
    let labels = window.query_all_by_role(egui::accesskit::Role::Label);
    labels.filter(|node| node.value().is_some_and(|name| name.ends_with(suffix))).count()
}

/// The count of rows on the screen with a freshness mark.
fn marks(window: &Window) -> usize {
    window.query_all_by_label_contains("old since ").filter(|node| node.rect().width() < 12.0).count()
}

/// The app on two fake clusters: the window, and the parts that must live as long as it.
struct Bed {
    window: Window,
    ams: Cluster,
    fra: Cluster,
    _home: Home,
    _runtime: Runtime,
}

/// Starts the app with the pods of the two clusters in its table.
fn bed() -> Bed {
    let runtime: Runtime = Builder::new_multi_thread().worker_threads(4).enable_all().build().expect("a runtime");
    let (ams, fra) = runtime.block_on(async { (cluster("-ams").await, cluster("-fra").await) });
    let home = home(&ams, &fra);
    let core = core(&runtime, &home);
    let mut window = window(core, egui::vec2(1280.0, 800.0));

    // The first start selects the cluster of `kubectl`. Its pods show with no command.
    until(&mut window, "the pods of ams", |window| shows(window, "25 pods"));
    assert!(rows(&window, "-ams") > 10 && rows(&window, "-fra") == 0 && marks(&window) == 0);

    // Weakness 1: a second cluster adds its pods to the same table.
    window.get_by_role_and_label(egui::accesskit::Role::CheckBox, "fra").click();
    until(&mut window, "the pods of the two clusters", |window| shows(window, "50 pods"));
    assert!(rows(&window, "-ams") > 0 && rows(&window, "-fra") > 0, "the rows of the two clusters mix by name");
    assert_eq!(marks(&window), 0, "each store is live");
    Bed { window, ams, fra, _home: home, _runtime: runtime }
}

/// Deletes the first pod of fra on its server and waits for the table with one pod less:
/// the watch of the store is open, and its changes arrive.
fn delete_a_pod_of_fra(window: &mut Window, fra: &Cluster) {
    let pods: Value = serde_json::from_str(&pod_list("-fra")).expect("JSON");
    let event = serde_json::json!({"type": "DELETED", "object": pods["items"][0]}).to_string();
    assert!(fra.server.send_event(PODS, &event) > 0, "the watch of the pods is open");
    until(window, "one pod less", |window| shows(window, "49 pods"));
}

/// Weaknesses 1, 7 and 8 of plan section 6, and the acceptance of milestone M1: the rows
/// of a cluster that is not live get their mark in 20 seconds or less.
#[test]
fn the_rows_of_a_cluster_with_a_silent_network_get_their_mark_in_20_seconds_or_less() {
    let Bed { mut window, ams: _ams, fra, _home, _runtime } = bed();

    // Weaknesses 7 and 8: the network of fra carries no byte and gives no error.
    fra.proxy.set(ProxyMode::Freeze);
    let marked = until(&mut window, "the mark on each row of fra", |window| marks(window) > 0);
    eprintln!("live: the rows of the silent cluster got their mark after {marked:.1?}");
    assert!(marked <= Duration::from_secs(20), "the mark came after {marked:?}");
    assert_eq!(marks(&window), rows(&window, "-fra"), "each row of fra has the mark, and no row of ams");
    assert!(rows(&window, "-fra") > 0 && shows(&window, "50 pods"), "the old rows stay (design rule 5)");
    // The note of the one cluster tells why. Its first text can be the failure of the
    // watch; then the store waits for a connection of its cluster.
    assert_eq!(window.query_all_by_label_contains("Pods: ").count(), 1);
    until(&mut window, "the store waits for a connection", |window| {
        window.query_all_by_label_contains("Pods: waits for a connection").count() == 1
    });
    assert_eq!(marks(&window), rows(&window, "-fra"), "the mark stays in each state that is not live");

    // The network returns: the rows are current again.
    fra.proxy.set(ProxyMode::Pass);
    let live = until(&mut window, "no mark", |window| marks(window) == 0);
    eprintln!("live: the rows were current again {live:.1?} after the network returned");
    assert!(live <= Duration::from_secs(10), "the rows were current after {live:?}");
    assert!(window.query_all_by_label_contains("Pods: ").next().is_none() && shows(&window, "50 pods"));

    // A pod that the cluster deletes leaves the table.
    delete_a_pod_of_fra(&mut window, &fra);
    stop_with_no_ended_task(&window);
}

/// Draws frames for this time. No frame shows a row with a freshness mark.
fn no_mark_for(window: &mut Window, time: Duration, why: &str) {
    let started = Instant::now();
    while started.elapsed() < time {
        window.run();
        assert_eq!(marks(window), 0, "{why}, after {:?}", started.elapsed());
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Draws frames until the window meets the condition, and then for `more`. In each
/// frame, each row of fra has its freshness mark: no row of fra looks current.
fn each_row_of_fra_is_old(window: &mut Window, what: &str, met: impl Fn(&Window) -> bool, more: Duration) {
    let started = Instant::now();
    let mut met_at = None;
    while met_at.is_none_or(|at: Instant| at.elapsed() < more) {
        window.run();
        let (marked, of_fra) = (marks(window), rows(window, "-fra"));
        assert!(
            marked > 0 && marked == of_fra,
            "{what}: {marked} of {of_fra} rows are old after {:?}",
            started.elapsed()
        );
        if met_at.is_none() && met(window) {
            met_at = Some(Instant::now());
        }
        assert!(started.elapsed() < LIMIT + more, "in time: {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// A slow peer, and a second fault during the recovery (plan section 13). A slow
/// cluster is not an old one: its rows get no mark while its watch renews through a
/// network with latency. A network that goes silent again while the store starts its
/// watch gives no row that looks current, at no moment.
#[test]
fn a_slow_cluster_has_no_mark_and_a_second_silence_during_the_recovery_keeps_each_mark() {
    let Bed { mut window, ams: _ams, fra, _home, _runtime } = bed();
    let slow = ProxyMode::Latency(Duration::from_millis(300));

    // Each byte of fra arrives 300 ms late, and the server ends each watch, as it does
    // at the timeout of a watch. The store stays live for the round trip of the renewal.
    fra.proxy.set(slow);
    for renewal in 1..=3 {
        no_mark_for(&mut window, Duration::from_millis(2500), "a slow cluster is live");
        // A bookmark shows that the watch works. The app takes the end of a watch that
        // gave no data in its first second as a fault, and a slow test machine must not
        // make that case.
        let version = 7 + renewal;
        let bookmark = serde_json::json!({
            "type": "BOOKMARK",
            "object": {"kind": "Pod", "apiVersion": "v1", "metadata": {"resourceVersion": version.to_string()}},
        });
        assert!(fra.server.send_event(PODS, &bookmark.to_string()) > 0, "renewal {renewal}: the watch is open");
        assert!(fra.server.end_watches() > 0, "renewal {renewal}: the server ends the watch");
    }
    no_mark_for(&mut window, Duration::from_millis(2500), "the last renewal");

    fra.proxy.set(ProxyMode::Freeze);
    let marked = until(&mut window, "the mark on each row of fra", |window| marks(window) > 0);
    eprintln!("live: the rows of the slow cluster got their mark {marked:.1?} after its network went silent");
    assert!(marked <= Duration::from_secs(20), "the mark came after {marked:?}");
    let waits = |window: &Window| window.query_all_by_label_contains("Pods: waits for a connection").count() == 1;
    each_row_of_fra_is_old(&mut window, "the store waits for a connection", waits, Duration::ZERO);

    // The network returns, and it is slow. It goes silent again in the first frame in
    // which the store does not wait for a connection any more: the request of its watch
    // is on its way then, and it ends at its time limit of 10 s.
    fra.proxy.set(slow);
    each_row_of_fra_is_old(&mut window, "the recovery starts", |window| !waits(window), Duration::ZERO);
    fra.proxy.set(ProxyMode::Freeze);
    each_row_of_fra_is_old(&mut window, "the second silence", |_| true, Duration::from_secs(13));
    assert!(shows(&window, "50 pods"), "the old rows stay (design rule 5)");

    fra.proxy.set(ProxyMode::Pass);
    let live = until(&mut window, "no mark", |window| marks(window) == 0);
    eprintln!(
        "live: after a second silence during the recovery, the rows were current {live:.1?} after the network returned"
    );
    assert!(live <= Duration::from_secs(10), "the rows were current after {live:?}");
    delete_a_pod_of_fra(&mut window, &fra);
    stop_with_no_ended_task(&window);
}
