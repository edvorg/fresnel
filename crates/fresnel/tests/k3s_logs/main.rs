//! The log views of milestone M9 on the real core and a real cluster (plan section 13): k3s
//! in Docker, the log sessions and the saves of the core, and the frames of the window.
//!
//! The fake core of the view tests builds each view by hand. This test proves the way from
//! the kubelet to the pane: the lines of a pod with the colors of its program, the search,
//! the full log in a file as `kubectl logs --timestamps` gives it, the log of the instance
//! before a crash, and the lines of each pod of a Deployment.

#![allow(clippy::expect_used, reason = "a test stops with a panic")]

#[path = "../common/window.rs"]
mod window;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui;
use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use fresnel_testkit::{Home, K3s};
use tokio::runtime::{Builder, Runtime};

use window::{Window, core, shows, stop_with_no_ended_task, until, until_within, window};

/// The namespace of the scene.
const SHOP: &str = "shop";
/// The image of the pods. The log test of `fresnel-core` uses it too.
const BUSYBOX: &str = "rancher/mirrored-library-busybox:1.36.1";
/// The longest wait for the scene: an image pull and the start of the pods.
const SCENE_LIMIT: Duration = Duration::from_secs(300);

/// A line in color each 200 ms: the program writes an SGR code before its level.
const TALK: &str = r#"i=0; while true; do printf '\033[32mINFO\033[0m talk %d\n' $i; i=$((i+1)); sleep 0.2; done"#;
/// The first run crashes, and each later run finds the mark of the first one in the volume.
const CRASH: &str = "if [ -f /mark/ran ]; then echo \"the next run\"; exec sleep 3600; fi; touch /mark/ran; \
                     echo \"first of the run\"; echo \"second of the run\"; exit 3";

/// A cluster with the scene, and the app on it. The fields drop in this sequence: the
/// window and its core go before the cluster.
struct Bed {
    window: Window,
    home: Home,
    k3s: K3s,
    runtime: Runtime,
}

impl Bed {
    fn kubectl(&self, arguments: &[&str]) -> String {
        let run = self.runtime.block_on(self.k3s.kubectl(arguments));
        run.unwrap_or_else(|error| panic!("kubectl {arguments:?}: {error}"))
    }
}

fn bed() -> Bed {
    let runtime: Runtime = Builder::new_multi_thread().worker_threads(4).enable_all().build().expect("a runtime");
    let k3s = runtime.block_on(K3s::start("m9-logs-view")).expect("k3s starts");
    let home = Home::new("m9-logs-view", &["kubeconfig.yaml"]).expect("a home folder");
    std::fs::copy(k3s.kubeconfig(), home.path("work/kubeconfig.yaml")).expect("the kubeconfig of the test cluster");
    // A tall window: the dock with a log view has room for its rows.
    let window = window(core(&runtime, &home), egui::vec2(1600.0, 1400.0));
    let bed = Bed { window, home, k3s, runtime };
    let container = serde_json::json!({"name": "crash", "image": BUSYBOX, "command": ["sh", "-c", CRASH],
        "volumeMounts": [{"name": "mark", "mountPath": "/mark"}]});
    let crash = serde_json::json!({"spec": {"volumes": [{"name": "mark", "emptyDir": {}}], "containers": [container]}});
    let crash = crash.to_string();
    let timeout = format!("--timeout={}s", SCENE_LIMIT.as_secs());
    let scene: &[&[&str]] = &[
        &["create", "namespace", SHOP],
        &["run", "talker", "--image", BUSYBOX, "-n", SHOP, "--", "sh", "-c", TALK],
        &["run", "crash", "--image", BUSYBOX, "--restart", "Always", "--overrides", &crash, "-n", SHOP],
        &[
            "create",
            "deployment",
            "chorus",
            "--image",
            BUSYBOX,
            "--replicas=2",
            "-n",
            SHOP,
            "--",
            "sh",
            "-c",
            "while true; do echo \"sing from $HOSTNAME\"; sleep 1; done",
        ],
        &["wait", "--for=condition=Ready", "pod/talker", "-n", SHOP, &timeout],
        &["rollout", "status", "deployment/chorus", "-n", SHOP, &timeout],
        &["wait", "--for=jsonpath={.status.containerStatuses[0].restartCount}=1", "pod/crash", "-n", SHOP, &timeout],
        &["wait", "--for=condition=Ready", "pod/crash", "-n", SHOP, &timeout],
    ];
    for step in scene {
        bed.kubectl(step);
    }
    bed
}

/// Opens the page of a kind and the panel of the object with this name. The filter of the
/// page has its own name: the dock with a log view has text fields too.
fn open(window: &mut Window, page: &str, filter: &str, name: &str) {
    window.get_by_role_and_label(Role::RadioButton, page).click();
    window.run();
    window.get_by_label(filter).click();
    window.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    window.get_by_label(filter).type_text(name);
    // The row and the heading of the panel have the name.
    let started = Instant::now();
    while window.query_all_by_label(name).count() < 2 {
        assert!(started.elapsed() < window::LIMIT, "in time: the panel of {name}");
        if let Some(row) = window.query_all_by_role_and_label(Role::Label, name).next() {
            row.click();
        }
        window.run();
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The texts of the rows of the pane on the screen.
fn rows(window: &Window) -> Vec<String> {
    let rows = window.query_all_by_role(Role::ListItem);
    rows.map(|node| node.accesskit_node().label().unwrap_or_default()).collect()
}

fn row_with(window: &Window, text: &str) -> bool {
    rows(window).iter().any(|row| row.contains(text))
}

/// Saves the full log of the open tab with the menu entry `entry`, and gives the text of
/// the file when the tab shows it is complete.
fn save_full(bed: &mut Bed, entry: &str) -> String {
    let before = files(&bed.home);
    bed.window.get_by_role_and_label(Role::Button, "Save").click();
    bed.window.run();
    bed.window.get_by_label(entry).click();
    until(&mut bed.window, "the save", |window| window.query_by_label_contains("Saved ").is_some());
    let new: Vec<PathBuf> = files(&bed.home).into_iter().filter(|file| !before.contains(file)).collect();
    assert_eq!(new.len(), 1, "one new file: {new:?}");
    std::fs::read_to_string(&new[0]).expect("the file")
}

/// The files of the folder of downloads of the test home.
fn files(home: &Home) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(home.path("Downloads")) else {
        return Vec::new();
    };
    entries.map(|entry| entry.expect("an entry").path()).collect()
}

#[test]
#[ignore = "needs k3s in Docker"]
fn the_log_views_show_the_lines_of_the_server_and_save_the_log_of_kubectl() {
    // The bed drops in the sequence of its fields, so no part of it moves out.
    let mut bed = bed();
    until(&mut bed.window, "the stores of the cluster", |window| shows(window, "Deployments"));

    // A pod: its lines with no escape sequence, the state of its source, and the search.
    open(&mut bed.window, "Pods", "Filter pods", "talker");
    bed.window.get_by_role_and_label(Role::Button, "Logs").click();
    let waited =
        until_within(&mut bed.window, "the lines of talker", SCENE_LIMIT, |window| row_with(window, "INFO talk 3"));
    eprintln!("k3s logs: the first lines showed after {waited:.1?}");
    assert!(shows(&bed.window, "talker/talker") && shows(&bed.window, "streaming"));
    assert!(rows(&bed.window).iter().all(|row| !row.contains('\u{1b}')), "the view has no escape sequence");
    bed.window.get_by_label("Search the log").click();
    bed.window.get_by_label("Search the log").type_text("talk 1");
    // A label of egui has its text as its value.
    let count = |window: &Window, part: &str| {
        let texts = window.query_all_by_label_contains(" hits").filter_map(|node| node.accesskit_node().value());
        texts.into_iter().any(|text| text.contains(part))
    };
    until(&mut bed.window, "the hits", |window| count(window, " hits"));
    bed.window.get_by_label("The next hit").click();
    until(&mut bed.window, "a hit in the pane", |window| count(window, " of "));
    assert!(row_with(&bed.window, "INFO talk 1"), "the pane shows the hit");

    // The full log in a file: each line of kubectl, with the time of the server.
    bed.window.get_by_role_and_label(Role::CheckBox, "Follow").click();
    until(&mut bed.window, "20 lines", |window| {
        let number = |row: &String| row.rsplit_once("INFO talk ").and_then(|(_, number)| number.parse::<u32>().ok());
        rows(window).iter().filter_map(number).any(|number| number >= 20)
    });
    let file = save_full(&mut bed, "Save the full log");
    let kubectl = bed.kubectl(&["logs", "talker", "-n", SHOP, "--timestamps"]);
    let (saved, read): (Vec<&str>, Vec<&str>) = (file.lines().collect(), kubectl.lines().collect());
    let both = saved.len().min(read.len());
    assert!(both >= 10, "the log has lines: {both}");
    assert_eq!(saved[..both], read[..both], "the file has the lines of kubectl logs --timestamps");
    assert!(saved[0].ends_with("\u{1b}[32mINFO\u{1b}[0m talk 0"), "the file keeps the bytes of the program");

    // The instance before the crash: its full log, as kubectl logs --previous gives it.
    open(&mut bed.window, "Pods", "Filter pods", "crash");
    let previous = bed.window.query_all_by_label("Previous logs of crash").next().expect("the button");
    previous.click_accesskit();
    until(&mut bed.window, "the earlier instance", |window| row_with(window, "second of the run"));
    let texts: Vec<String> =
        rows(&bed.window).into_iter().filter_map(|row| row.split_once(' ').map(|(_, text)| text.to_owned())).collect();
    assert_eq!(texts, ["first of the run", "second of the run"]);
    assert!(shows(&bed.window, "ended: the view has the full log of the earlier instance"));
    let file = save_full(&mut bed, "Save the full log of the previous instance");
    let kubectl = bed.kubectl(&["logs", "crash", "-n", SHOP, "--previous", "--timestamps"]);
    assert_eq!(file.lines().collect::<Vec<_>>(), kubectl.lines().collect::<Vec<_>>());
    assert_eq!(file.lines().count(), 2, "the two lines of the run before the crash");

    // A Deployment: the lines of each of its pods, each one with its source.
    open(&mut bed.window, "Deployments", "Filter Deployments", "chorus");
    bed.window.get_by_role_and_label(Role::Button, "Logs").click();
    let pods =
        bed.kubectl(&["get", "pods", "-n", SHOP, "-l", "app=chorus", "-o", "jsonpath={.items[*].metadata.name}"]);
    let pods: Vec<String> = pods.split_whitespace().map(str::to_owned).collect();
    assert_eq!(pods.len(), 2);
    let container = bed.kubectl(&[
        "get",
        "deployment",
        "chorus",
        "-n",
        SHOP,
        "-o",
        "jsonpath={.spec.template.spec.containers[0].name}",
    ]);
    until(&mut bed.window, "the lines of both pods", |window| {
        pods.iter().all(|pod| row_with(window, &format!("{pod}/{} sing from {pod}", container.trim())))
    });
    stop_with_no_ended_task(&bed.window);
}
