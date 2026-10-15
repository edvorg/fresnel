//! Two k3s clusters with the same objects, a user of cluster `b` who may only read its
//! Deployments, and the app core with the contexts `a`, `b` and `viewer` (`b` as that user).
//! The core starts as from Finder and reads the kubeconfig of a home folder of the test.

use std::sync::Arc;
use std::time::Duration;

use fresnel_core::app::{ClusterData, Command, Core, CoreParts, CoreSettings, CoreView, JobView};
use fresnel_core::cluster::{ClusterId, Link, Registry};
use fresnel_core::journal::{Entry, Journal, Level, TASK_SOURCE};
use fresnel_core::kinds::{Describe, Kind, Object};
use fresnel_core::net::{NetSettings, WallClock};
use fresnel_core::ops::{JobState, Order, Outcome, Target};
use fresnel_core::published::Held;
use fresnel_core::store::{ObjectKey, Snapshot, Written};
use fresnel_testkit::{Home, K3s};
use tokio::runtime::Handle;
use tokio::sync::Notify;
use tokio::time::timeout;

use crate::common::{TestTasks, connect, registry, until_live};
use crate::contexts::kubeconfig;
use crate::scene::{self, APP};

/// The longest wait for a state of the core or of a cluster.
pub(crate) const LIMIT: Duration = Duration::from_secs(180);
/// The kinds whose full objects the test reads in the view of the core, as a page shows them.
const SHOWN: [Kind; 4] = [Kind::Deployment, Kind::CronJob, Kind::Job, Kind::Secret];

/// The core and the clusters. The fields drop in this sequence: the core stops before the
/// connection of the setup, and both before the containers.
pub(crate) struct Bed {
    pub(crate) core: Core,
    /// The wake of the UI: the core asks for a repaint.
    woken: Arc<Notify>,
    _home: Home,
    _setup: (Link, Registry, TestTasks),
    pub(crate) a: K3s,
    pub(crate) b: K3s,
}

impl Bed {
    pub(crate) async fn start() -> Self {
        let (a, b) = tokio::join!(K3s::start("app-a"), K3s::start("app-b"));
        let (a, b) = (a.expect("k3s a starts"), b.expect("k3s b starts"));
        tokio::join!(scene::create(&a), scene::create(&b));
        // The cluster admin of b makes the user `viewer` with its Role.
        let tasks = TestTasks::new(&Handle::current());
        let setup = registry(&tasks, &Handle::current(), &b.kubeconfig(), NetSettings::default());
        let admin = connect(&setup, "default");
        until_live(&admin).await;
        let viewer = crate::access::viewer(&admin.gateway, &b).await;

        let home = Home::new("k3s-app", &["kubeconfig.yaml"]).expect("a home folder");
        let contexts = [("a", a.kubeconfig()), ("b", b.kubeconfig()), ("viewer", viewer)];
        home.write("work/kubeconfig.yaml", &kubeconfig(&contexts)).expect("the kubeconfig");
        let core = Core::new(CoreParts {
            runtime: Handle::current(),
            base_env: home.finder_env(),
            journal: Journal::new(10_000, WallClock::system()),
            state_file: home.path("state/state.json"),
            config_file: home.path("state/config.toml"),
            detectors: Vec::new(),
            settings: CoreSettings::default(),
        });
        let woken = Arc::new(Notify::new());
        core.start({
            let woken = Arc::clone(&woken);
            move || woken.notify_one()
        });
        let bed = Self { core, woken, _home: home, _setup: (admin, setup, tasks), a, b };
        // The first start selects the current context `a`, as for a user with no state file.
        for cluster in ["b", "viewer"] {
            bed.core.send(Command::Select { cluster: ClusterId::new(cluster), selected: true });
        }
        bed.core.send(Command::ShowKinds(SHOWN.into_iter().collect()));
        bed.until("the stores of a and b have the objects of the test", |view| {
            ["a", "b"].iter().all(|cluster| scene::is_loaded(view, cluster)) && data(view, "viewer").is_some()
        })
        .await;
        bed
    }

    /// Waits until the view meets the condition. The wait reads the view again at each
    /// wake of the UI, as the window does.
    pub(crate) async fn until(&self, what: &str, met: impl Fn(&CoreView) -> bool) -> CoreView {
        let wait = async {
            loop {
                let view = self.core.view();
                if met(&view) {
                    return view;
                }
                self.woken.notified().await;
            }
        };
        timeout(LIMIT, wait).await.unwrap_or_else(|_| {
            let view = self.core.view();
            let jobs: Vec<&JobView> = view.jobs.jobs.iter().map(|job| &**job).collect();
            let journal: Vec<String> = view.journal.entries.iter().map(|entry| line(entry)).collect();
            panic!("in time: {what}\njobs: {jobs:#?}\njournal: {journal:#?}\nclusters: {:#?}", view.clusters)
        })
    }

    /// The target of an object of the namespace `app` with its UID in the store of the core,
    /// as a view takes it from the row that the user selected.
    pub(crate) fn target<K: Describe>(&self, cluster: &str, name: &str) -> Target {
        let key = ObjectKey::new(APP, name);
        let snapshot = snapshot::<K>(&self.core.view(), cluster).expect("a store of the kind");
        let object = snapshot.items.get(&key).unwrap_or_else(|| panic!("{name} is in the store of {cluster}"));
        let uid = object.meta().uid.clone().expect("a UID");
        Target { cluster: ClusterId::new(cluster), kind: K::ID, key, uid: uid.into() }
    }

    /// Sends one `Command::Run` with these orders, as the confirm dialog does. Gives the
    /// number of its first job: each new job of the core has the next number.
    pub(crate) fn send(&self, orders: Vec<Order>) -> u64 {
        let first = self.core.view().jobs.jobs.last().map_or(1, |job| job.number + 1);
        self.core.send(Command::Run(orders));
        first
    }

    /// Sends one `Command::Run` with these orders and waits for the final state of each of
    /// its jobs. Gives the jobs in the sequence of the orders.
    pub(crate) async fn run(&self, orders: Vec<Order>) -> Vec<Arc<JobView>> {
        let count = orders.len();
        let first = self.send(orders.clone());
        let ended = |view: &CoreView| {
            let jobs = jobs_from(view, first);
            jobs.len() == count && jobs.iter().all(|job| job.report.state.is_final())
        };
        let jobs = jobs_from(&self.until("the end of each job of the command", ended).await, first);
        let ran: Vec<&Order> = jobs.iter().map(|job| &job.order).collect();
        assert_eq!(ran, orders.iter().collect::<Vec<_>>(), "one job for each order of the command");
        jobs
    }

    /// Runs one order as one command to the final state of its job.
    pub(crate) async fn run_one(&self, order: Order) -> Arc<JobView> {
        let mut jobs = self.run(vec![order]).await;
        jobs.pop().expect("the job of the order")
    }

    /// Asserts the mark "change pending" of a write: the store of `before` the command shows
    /// it on the object of the write, and the store of the core drops it when the watch
    /// event of the write arrives, as the panel of the object shows it.
    pub(crate) async fn mark<K: Describe>(&self, cluster: &str, before: &Snapshot<K>, written: &Written) {
        assert!(before.is_pending(written), "the store of before the write shows the mark: {written:?}");
        self.until("the watch event of the write ends the mark", |view| {
            snapshot::<K>(view, cluster).is_some_and(|now| !now.is_pending(written))
        })
        .await;
    }

    /// Asserts that no task of the core failed or ended where it must not: the jobs end as
    /// events of the journal, and each other task runs to the stop.
    pub(crate) fn no_task_failed(&self) {
        let view = self.core.view();
        let ended =
            view.journal.entries.iter().filter(|entry| entry.source == TASK_SOURCE && entry.level != Level::Info);
        let ended: Vec<String> = ended.map(|entry| line(entry)).collect();
        assert_eq!(ended, Vec::<String>::new(), "a task of the core failed");
    }
}

/// The jobs of the core from the job with this number on.
fn jobs_from(view: &CoreView, first: u64) -> Vec<Arc<JobView>> {
    view.jobs.jobs.iter().filter(|job| job.number >= first).cloned().collect()
}

/// The job with this number.
pub(crate) fn job(view: &CoreView, number: u64) -> Option<&JobView> {
    view.jobs.jobs.iter().map(|job| &**job).find(|job| job.number == number)
}

/// The data of a connected cluster in the view.
pub(crate) fn data<'a>(view: &'a CoreView, cluster: &str) -> Option<&'a ClusterData> {
    view.clusters.iter().find(|row| row.id.as_str() == cluster)?.data.as_ref()
}

/// The snapshot of the full objects of a kind in a cluster, as a page reads it.
pub(crate) fn snapshot<K: Describe>(view: &CoreView, cluster: &str) -> Option<Held<Snapshot<K>>> {
    data(view, cluster)?.snapshot::<K>()
}

/// The job completed, and its outcome is the answer of a write.
pub(crate) fn written(job: &JobView) -> &Written {
    assert_eq!(job.report.state, JobState::Done, "the job: {job:#?}");
    match &job.report.outcome {
        Outcome::Written(Some(written)) => written,
        other => panic!("the answer of a write: {other:?}"),
    }
}

/// One entry of the journal as one text.
fn line(entry: &Entry) -> String {
    format!("{:?} {}: {}", entry.level, entry.source, entry.text)
}
