//! The app core on the fake API server, started as from Finder.
//!
//! Each test gives the core a home folder of its own with a shell profile. The login shell
//! of the user reads that profile, so the core finds the kubeconfig and the plugin as the
//! installed app does.
//!
//! - `start`: the start and the stop. `commands`: the selection and "Retry now".
//! - `network`: a network that goes away and comes back.

mod commands;
mod network;
mod start;

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use fresnel_core::app::{ClusterRow, Command, Core, CoreParts, CoreSettings, CoreView, Health, Startup};
use fresnel_core::cluster::ClusterId;
use fresnel_core::journal::{Journal, Level};
use fresnel_core::net::WallClock;
use fresnel_core::net::detect::{Detector, Disturbance};
use fresnel_testkit::{FakeApiServer, Home};
use tokio::runtime::Handle;
use tokio::sync::Notify;
use tokio::time::timeout;

use crate::bed::authority;
use crate::common::LIMIT;

/// The command of the plugin in the kubeconfig: a bare name, which only the profile finds.
const PLUGIN: &str = "fresnel-test-plugin";
const KUBECONFIG: &str = "work/kubeconfig.yaml";

/// A home folder with the plugin of the tests of `fresnel-net` in its folder `plugins`.
fn home(label: &str) -> Home {
    let home = Home::new(label, &["kubeconfig.yaml"]).expect("a home folder");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fresnel-net/tests/fixtures/fake-plugin");
    home.add_program(PLUGIN, &format!("#!/bin/sh\nexec '{}'\n", fixture.display())).expect("the plugin");
    home
}

/// One context of the test kubeconfig: its name, its address, its server and if its user
/// has the plugin. The address is the server or a proxy before it.
type Entry<'a> = (&'a str, SocketAddr, &'a FakeApiServer, bool);

/// Writes the kubeconfig of the home folder, with `current` as its current context.
fn write_kubeconfig(home: &Home, current: &str, entries: &[Entry<'_>]) {
    let (mut clusters, mut users, mut contexts) = (String::new(), String::new(), String::new());
    let state = home.path("state");
    for (name, address, server, plugin) in entries {
        let authority = authority(server);
        clusters.push_str(&format!(
            "- {{name: {name}, cluster: {{server: 'https://{address}', certificate-authority-data: {authority}}}}}\n"
        ));
        if *plugin {
            users.push_str(&format!(
                "- name: {name}
  user:
    exec:
      apiVersion: client.authentication.k8s.io/v1beta1
      command: {PLUGIN}
      env: [{{name: FAKE_PLUGIN_STATE, value: '{}'}}, {{name: FAKE_PLUGIN_TOKEN, value: core}}]
",
                state.display()
            ));
            contexts.push_str(&format!("- {{name: {name}, context: {{cluster: {name}, user: {name}}}}}\n"));
        } else {
            contexts.push_str(&format!("- {{name: {name}, context: {{cluster: {name}}}}}\n"));
        }
    }
    let text = format!("current-context: {current}\nclusters:\n{clusters}users:\n{users}contexts:\n{contexts}");
    home.write(KUBECONFIG, &text).expect("the kubeconfig");
}

/// A detector that sees a network change when the test sets its flag.
struct OnOrder(Arc<AtomicBool>);

impl Detector for OnOrder {
    fn look(&mut self) -> Option<Disturbance> {
        self.0.swap(false, Ordering::Relaxed).then_some(Disturbance::NetworkChange)
    }
}

/// The core of one test. A task of the core that ends is a defect, and the drop fails the
/// test for it.
struct App {
    core: Core,
    /// The wake of the UI: the core asks for a repaint.
    woken: Arc<Notify>,
    network_changed: Arc<AtomicBool>,
}

impl App {
    /// Builds the core with the environment of a program from Finder. It is not started.
    fn new(home: &Home, mut settings: CoreSettings) -> Self {
        settings.detect_period = Duration::from_millis(20);
        let network_changed = Arc::new(AtomicBool::new(false));
        let core = Core::new(CoreParts {
            runtime: Handle::current(),
            base_env: home.finder_env(),
            journal: Journal::new(1000, WallClock::system()),
            detectors: vec![Box::new(OnOrder(Arc::clone(&network_changed)))],
            settings,
        });
        Self { core, woken: Arc::new(Notify::new()), network_changed }
    }

    fn start(&self) {
        let woken = Arc::clone(&self.woken);
        self.core.start(move || woken.notify_one());
    }

    fn started(home: &Home, settings: CoreSettings) -> Self {
        let app = Self::new(home, settings);
        app.start();
        app
    }

    /// Waits until the view meets the condition. The wait reads the view again at each
    /// wake of the UI, as the window does.
    async fn until(&self, what: &str, met: impl Fn(&CoreView) -> bool) -> CoreView {
        let wait = async {
            loop {
                let view = self.core.view();
                if met(&view) {
                    return view;
                }
                self.woken.notified().await;
            }
        };
        timeout(LIMIT, wait).await.unwrap_or_else(|_| panic!("in time: {what}: {:#?}", self.core.view()))
    }

    async fn until_health(&self, cluster: &str, health: Health) -> CoreView {
        self.until(&format!("{cluster} is {health:?}"), |view| {
            view.startup == Startup::Ready && row(view, cluster).health() == health
        })
        .await
    }

    fn select(&self, cluster: &str, selected: bool) {
        self.core.send(Command::Select { cluster: ClusterId::new(cluster), selected });
    }

    /// The entries of the journal as (level, source, text).
    fn journal(&self) -> Vec<(Level, String, String)> {
        let view = self.core.view().journal;
        view.entries.iter().map(|entry| (entry.level, entry.source.clone(), entry.text.clone())).collect()
    }

    /// The texts of the entries of one source.
    fn texts_of(&self, source: &str) -> Vec<String> {
        self.journal().into_iter().filter(|(_, from, _)| from == source).map(|(_, _, text)| text).collect()
    }
}

impl Drop for App {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            assert_eq!(self.texts_of("task"), Vec::<String>::new(), "a task of the app ended");
        }
    }
}

fn row<'a>(view: &'a CoreView, cluster: &str) -> &'a ClusterRow {
    let found = view.clusters.iter().find(|row| row.id.as_str() == cluster);
    found.unwrap_or_else(|| panic!("the view has the cluster {cluster}: {view:#?}"))
}

fn states(view: &CoreView) -> Vec<(&str, bool, Health)> {
    view.clusters.iter().map(|row| (row.id.as_str(), row.selected, row.health())).collect()
}
