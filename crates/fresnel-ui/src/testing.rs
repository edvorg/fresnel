//! Helpers of the unit tests: the task start function with the reports of its tasks.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, SystemTime};

use fresnel_core::kinds::{Describe, Object};
use fresnel_core::net::task::{TaskReport, Tasks};
use fresnel_core::published::{Disposal, Held, Notice, Publisher};
use fresnel_core::store::{Snapshot, StoreState};
use k8s_openapi::api::core::v1::Pod;
use tokio::runtime::{Builder, Runtime};

/// The longest wait for a signal that must arrive.
pub(crate) const LIMIT: Duration = Duration::from_secs(30);
/// The wait of a check that nothing arrives.
pub(crate) const QUIET: Duration = Duration::from_millis(100);

/// The task start function of one test. A worker thread needs no runtime, but the
/// function takes the handle of one for its other tasks.
pub(crate) struct TestTasks {
    pub(crate) tasks: Tasks,
    /// One report for each run of a task that ended.
    pub(crate) reports: Receiver<TaskReport>,
    disposal: Disposal,
    _runtime: Runtime,
}

impl TestTasks {
    pub(crate) fn new() -> Self {
        let runtime = Builder::new_current_thread().build().expect("a runtime");
        let (sender, reports) = channel();
        let tasks = Tasks::new(runtime.handle().clone(), move |report: TaskReport| {
            // The test can end before a thread that it stopped.
            if sender.send(report).is_err() {}
        });
        let disposal = Disposal::start(&tasks);
        Self { tasks, reports, disposal, _runtime: runtime }
    }

    /// A value as a reader of a published state holds it.
    pub(crate) fn held<T: Send + Sync + 'static>(&self, value: T) -> Held<T> {
        Publisher::new(value, self.disposal.bin(), Notice::new()).read()
    }
}

/// The pods of the scene on k3s that `fresnel-core` keeps as its fixture: one pod for
/// each state of the status rule. No object of a real cluster is in the repository.
pub(crate) fn fixture_pods() -> Vec<Arc<Pod>> {
    let text = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../fresnel-core/fixtures/k3s/pods.json"));
    serde_json::from_str::<Vec<Pod>>(text).expect("the fixture").into_iter().map(Arc::new).collect()
}

/// Each object of a kind of tier A in the same scene.
pub(crate) fn fixture<K: Describe>() -> Vec<Arc<K>> {
    let folder = concat!(env!("CARGO_MANIFEST_DIR"), "/../fresnel-core/fixtures/k3s");
    let text = std::fs::read_to_string(format!("{folder}/{}.json", K::ID.info().plural)).expect("a fixture");
    serde_json::from_str::<Vec<K>>(&text).expect("the fixture").into_iter().map(Arc::new).collect()
}

/// The snapshot of a store with these objects, after a complete list.
pub(crate) fn snapshot<T: Object>(pods: &[Arc<T>], state: StoreState) -> Snapshot<T> {
    Snapshot {
        generation: 1,
        state,
        since: SystemTime::UNIX_EPOCH,
        last_contact: Some(SystemTime::UNIX_EPOCH),
        listed: true,
        dropped: 0,
        items: pods.iter().cloned().collect(),
    }
}
