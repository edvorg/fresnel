//! The helpers that each test of `fresnel-core` uses.

#![allow(clippy::expect_used, reason = "a test stops with a panic")]

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use fresnel_core::cluster::{ClusterId, Link, Registry};
use fresnel_core::kubeconfig;
use fresnel_core::net::env::ProcessEnv;
use fresnel_core::net::task::{TaskReport, Tasks};
use fresnel_core::net::{ChannelState, ClusterStatus, NetSettings};
use tokio::runtime::Handle;
use tokio::time::timeout;

/// The longest wait for a signal that must arrive.
pub(crate) const LIMIT: Duration = Duration::from_secs(30);

pub(crate) async fn in_time<T>(what: &str, signal: impl Future<Output = T>) -> T {
    timeout(LIMIT, signal).await.unwrap_or_else(|_| panic!("in time: {what}"))
}

/// The tasks of the clusters of one test. A task that ends is a defect, and the drop of
/// this value fails the test for it.
pub(crate) struct TestTasks {
    pub(crate) tasks: Tasks,
    ended: Arc<Mutex<Vec<String>>>,
}

impl TestTasks {
    pub(crate) fn new(runtime: &Handle) -> Self {
        let ended = Arc::new(Mutex::new(Vec::new()));
        let reports = Arc::clone(&ended);
        let tasks = Tasks::new(runtime.clone(), move |report: TaskReport| {
            reports.lock().expect("a lock").push(report.to_string());
        });
        Self { tasks, ended }
    }
}

impl Drop for TestTasks {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            assert_eq!(*self.ended.lock().expect("a lock"), Vec::<String>::new(), "a task of the app ended");
        }
    }
}

/// A registry with the contexts of one kubeconfig file, in the environment of a program
/// from Finder.
pub(crate) fn registry(tasks: &TestTasks, runtime: &Handle, kubeconfig: &Path, settings: NetSettings) -> Registry {
    let env: ProcessEnv = [("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")].into_iter().collect();
    let registry = Registry::new(tasks.tasks.clone(), runtime.clone(), env, settings);
    let loaded = kubeconfig::load(&[kubeconfig.to_owned()]);
    assert_eq!(loaded.problems, [], "the kubeconfig of the test");
    registry.set_contexts(loaded.contexts);
    registry
}

/// Connects one cluster of the registry.
pub(crate) fn connect(registry: &Registry, context: &str) -> Link {
    registry.connect(&ClusterId::new(context)).unwrap_or_else(|error| panic!("{context} connects: {error}"))
}

/// Waits until each first channel of the cluster is live.
pub(crate) async fn until_live(link: &Link) -> ClusterStatus {
    let mut changes = link.net.changes();
    let live = async {
        loop {
            let status = link.net.status();
            let groups = [&status.watches, &status.requests, &status.logs];
            if groups.iter().all(|group| group.channels.iter().all(|channel| channel.state == ChannelState::Live)) {
                return status;
            }
            changes.next().await;
        }
    };
    timeout(LIMIT, live).await.unwrap_or_else(|_| panic!("each channel is live in time: {:#?}", link.net.status()))
}
