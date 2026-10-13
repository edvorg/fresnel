//! The views of the app on its real core, with no window (plan sections 6 and 15).
//!
//! The core reads the pods of two fake API servers through fault proxies, and the test
//! draws the frames of the window with [`fresnel::frame::draw`]. So the test covers the
//! way from a fault of the network to the freshness mark of a row. It uses real time, and
//! each wait ends at a frame that shows what the test waits for.

#![allow(clippy::expect_used, reason = "a test stops with a panic")]

#[path = "../common/mod.rs"]
mod common;

use std::time::{Duration, Instant};

use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use fresnel_core::app::{Core, CoreParts, CoreSettings};
use fresnel_core::journal::{Journal, TASK_SOURCE};
use fresnel_core::net::WallClock;
use fresnel_testkit::{FakeApiServer, FaultProxy, Home, ProxyMode};
use fresnel_ui::{Clock, Shell};
use serde_json::Value;
use tokio::runtime::{Builder, Runtime};

/// The longest wait for a frame that must come.
const LIMIT: Duration = Duration::from_secs(60);
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

/// What a frame draws: the shell of the app on the core of the app.
struct App {
    core: Core,
    shell: Option<Shell>,
}

type Window = Harness<'static, App>;

fn window(core: Core) -> Window {
    let frame = |ui: &mut egui::Ui, app: &mut App| {
        let core = &app.core;
        let shell = app.shell.get_or_insert_with(|| Shell::new(ui.ctx(), Clock::system(), &core.tasks()));
        fresnel::frame::draw(shell, core, ui);
    };
    let builder = Harness::builder().with_size([1280.0, 800.0]).with_step_dt(1.0 / 60.0).with_max_steps(64);
    let window = builder.build_ui_state(frame, App { core, shell: None });
    // The core starts after the first frame, as in the window of the app.
    let ctx = window.ctx.clone();
    window.state().core.start(move || ctx.request_repaint());
    window
}

/// Draws frames until the window meets the condition. Gives the time of the wait.
fn until(window: &mut Window, what: &str, met: impl Fn(&Window) -> bool) -> Duration {
    let started = Instant::now();
    loop {
        window.run();
        if met(window) {
            return started.elapsed();
        }
        let journal = || window.state().core.view().journal;
        assert!(started.elapsed() < LIMIT, "in time: {what}: {:#?}", journal().entries);
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn shows(window: &Window, text: &str) -> bool {
    window.query_all_by_label(text).next().is_some()
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

/// Weaknesses 1, 7 and 8 of plan section 6, and the acceptance of milestone M1: the rows
/// of a cluster that is not live get their mark in 20 seconds or less.
#[test]
fn the_rows_of_a_cluster_with_a_silent_network_get_their_mark_in_20_seconds_or_less() {
    let runtime: Runtime = Builder::new_multi_thread().worker_threads(4).enable_all().build().expect("a runtime");
    let (ams, fra) = runtime.block_on(async { (cluster("-ams").await, cluster("-fra").await) });
    let home = home(&ams, &fra);
    let core = Core::new(CoreParts {
        runtime: runtime.handle().clone(),
        base_env: home.finder_env(),
        journal: Journal::new(1000, WallClock::system()),
        state_file: home.path("state/state.json"),
        detectors: Vec::new(),
        settings: CoreSettings::default(),
    });
    let mut window = window(core);

    // The first start selects the cluster of `kubectl`. Its pods show with no command.
    until(&mut window, "the pods of ams", |window| shows(window, "25 pods"));
    assert!(rows(&window, "-ams") > 10 && rows(&window, "-fra") == 0 && marks(&window) == 0);

    // Weakness 1: a second cluster adds its pods to the same table.
    window.get_by_role_and_label(egui::accesskit::Role::CheckBox, "fra").click();
    until(&mut window, "the pods of the two clusters", |window| shows(window, "50 pods"));
    assert!(rows(&window, "-ams") > 0 && rows(&window, "-fra") > 0, "the rows of the two clusters mix by name");
    assert_eq!(marks(&window), 0, "each store is live");

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
    let gone = pod_list("-fra");
    let gone: Value = serde_json::from_str(&gone).expect("JSON");
    let event = serde_json::json!({"type": "DELETED", "object": gone["items"][0]}).to_string();
    assert!(fra.server.send_event(PODS, &event) > 0, "the watch of the pods is open again");
    until(&mut window, "one pod less", |window| shows(window, "49 pods"));

    let core = &window.state().core;
    let ended: Vec<String> = core
        .view()
        .journal
        .entries
        .iter()
        .filter(|entry| entry.source == TASK_SOURCE)
        .map(|entry| entry.text.clone())
        .collect();
    assert_eq!(ended, Vec::<String>::new(), "no task of the app ended");
    core.stop();
}
