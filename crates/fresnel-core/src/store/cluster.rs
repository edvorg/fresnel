//! The stores of one cluster: one for each kind of tier A and tier B.

use std::any::Any;
use std::sync::Arc;
use std::time::Duration;

use fresnel_net::task::{Backoff, Restart, TaskHandle, TaskName, Tasks};
use fresnel_net::{Signal, WallClock};
use tokio::sync::watch::Sender;

use super::snapshot::{Snapshot, StoreStatus};
use super::state::{StoreState, Wait};
use crate::cluster::{ClusterId, Gateway, Link};
use crate::error::CoreError;
use crate::kinds::{Full, Kind, Meta, Object, Visitor, each_store};
use crate::published::{Bin, Changes, Notice, Publisher, Reader};
use crate::watch::{self, GatewaySource, LaneStates, Lanes, ListGate, Parts, WatchSettings};

/// Which Events a cluster keeps (plan section 9.6). Events can be most of the objects of
/// a cluster.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EventsMode {
    /// Each Event.
    #[default]
    All,
    /// Only the Events of the type `Warning`. The server selects them.
    Warnings,
    /// No store. A view reads the Events of its object while it needs them.
    OnDemand,
}

/// The settings of the stores of one cluster.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoreSettings {
    /// The watch loop of each store.
    pub watch: WatchSettings,
    /// Which Events the cluster keeps.
    pub events: EventsMode,
    /// The most Events that the cluster keeps. The oldest Event leaves first.
    pub max_events: usize,
    /// The pauses before the task of a store starts again after a fault of its own.
    pub restart: Backoff,
}

impl Default for StoreSettings {
    /// A task that fails at each start gives one journal entry for each run. With these
    /// pauses, a broken store writes two entries in one minute at most.
    fn default() -> Self {
        let restart = Backoff {
            first: Duration::from_millis(500),
            max: Duration::from_secs(30),
            jitter_percent: 50,
            reset_after: Duration::from_secs(60),
        };
        Self { watch: WatchSettings::default(), events: EventsMode::default(), max_events: 20_000, restart }
    }
}

/// What the owner of the stores gives to them.
#[derive(Clone)]
pub struct StoreParts {
    /// The settings.
    pub settings: StoreSettings,
    /// Frees the snapshots that the readers held.
    pub bin: Bin,
    /// The wall clock, for the times that a state shows.
    pub clock: WallClock,
}

/// One store with no type of its objects.
trait AnyStore: Send + Sync {
    fn status(&self) -> StoreStatus;

    /// Publishes the final state. What the task publishes after it does not show.
    fn stop(&self);

    fn as_any(&self) -> &dyn Any;
}

struct Store<T> {
    kind: Kind,
    publisher: Publisher<Snapshot<T>>,
    clock: WallClock,
}

impl<T: Object> AnyStore for Store<T> {
    fn status(&self) -> StoreStatus {
        self.publisher.read().status(self.kind)
    }

    fn stop(&self) {
        self.publisher.close(|last| last.in_state(StoreState::Stopped, self.clock.now()));
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// A store with its task. The task stops when this value drops, and the readers of the
/// store then see the state "stopped" with the last objects.
struct Running {
    kind: Kind,
    store: Box<dyn AnyStore>,
    _task: TaskHandle,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.store.stop();
    }
}

/// The stores of one cluster. Each one stops when this value drops.
pub struct ClusterStores {
    cluster: ClusterId,
    stores: Vec<Running>,
    notice: Notice,
    _lanes: TaskHandle,
}

impl ClusterStores {
    /// Starts the store of each kind of tier A and tier B, with one task for each store.
    /// The first lists run in the sequence of the kinds, four at a time.
    pub fn start(tasks: &Tasks, link: &Link, parts: StoreParts) -> Result<Self, CoreError> {
        if let Some(problem) = parts.settings.watch.problem() {
            return Err(problem);
        }
        let cluster = link.gateway.cluster().clone();
        let states = Sender::new(LaneStates::UNKNOWN);
        let lanes = Lanes::new(states.subscribe());
        let lanes_task =
            tasks.start(TaskName::scoped("store lanes", cluster.as_str()), Restart::Always(Backoff::default()), {
                let net = link.net.clone();
                move || watch::follow(net.clone(), states.clone())
            });
        let mut starter = Starter {
            tasks,
            gateway: &link.gateway,
            signal: link.net.signal(),
            lanes,
            gate: ListGate::new(parts.settings.watch.lists),
            watch: Arc::new(parts.settings.watch.clone()),
            parts,
            notice: Notice::new(),
            stores: Vec::new(),
        };
        each_store(&mut starter);
        Ok(Self { cluster, stores: starter.stores, notice: starter.notice, _lanes: lanes_task })
    }

    /// The cluster of the stores.
    pub fn cluster(&self) -> &ClusterId {
        &self.cluster
    }

    /// Tells when a store of the cluster has a new snapshot: new objects or a new state.
    pub fn changes(&self) -> Changes {
        self.notice.changes()
    }

    /// The facts of each store, in the sequence of the kinds.
    pub fn statuses(&self) -> Vec<StoreStatus> {
        self.stores.iter().map(|running| running.store.status()).collect()
    }

    /// A reader of the store of a tier A kind, with its full objects. `None` for the Events
    /// when the settings give them no store.
    pub fn full<K: Full>(&self) -> Option<Reader<Snapshot<K>>> {
        self.reader(K::KIND)
    }

    /// A reader of the store of a tier B kind, with the metadata of its objects. `None`
    /// for a kind of an other tier.
    pub fn metadata(&self, kind: Kind) -> Option<Reader<Snapshot<Meta>>> {
        self.reader(kind)
    }

    fn reader<T: Object>(&self, kind: Kind) -> Option<Reader<Snapshot<T>>> {
        let running = self.stores.iter().find(|running| running.kind == kind)?;
        let store = running.store.as_any().downcast_ref::<Store<T>>()?;
        Some(store.publisher.reader())
    }
}

impl std::fmt::Debug for ClusterStores {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let stores = self.stores.len();
        f.debug_struct("ClusterStores").field("cluster", &self.cluster).field("stores", &stores).finish()
    }
}

/// Starts the store of each kind that the list of the kinds gives.
struct Starter<'a> {
    tasks: &'a Tasks,
    gateway: &'a Gateway,
    signal: Signal,
    lanes: Lanes,
    gate: Arc<ListGate>,
    watch: Arc<WatchSettings>,
    parts: StoreParts,
    notice: Notice,
    stores: Vec<Running>,
}

/// The field selector for the Events of the type `Warning`.
const WARNINGS: &str = "type=Warning";

impl Visitor for Starter<'_> {
    fn full<K: Full>(&mut self) {
        let fields = match (K::KIND, self.parts.settings.events) {
            (Kind::Event, EventsMode::OnDemand) => return,
            (Kind::Event, EventsMode::Warnings) => Some(WARNINGS),
            _ => None,
        };
        let max_items = (K::KIND == Kind::Event).then_some(self.parts.settings.max_events);
        self.start::<K>(K::KIND, fields, max_items);
    }

    fn metadata(&mut self, kind: Kind) {
        self.start::<Meta>(kind, None, None);
    }
}

impl Starter<'_> {
    fn start<T: Object>(&mut self, kind: Kind, fields: Option<&'static str>, max_items: Option<usize>) {
        let clock = self.parts.clock.clone();
        let first = Snapshot::<T>::empty(StoreState::Waiting(Wait::Channel), clock.now());
        let publisher = Publisher::new(first, self.parts.bin.clone(), self.notice.clone());
        let parts = Arc::new(Parts {
            kind,
            source: GatewaySource::new(self.gateway.clone(), kind.info(), fields),
            publisher: publisher.clone(),
            lanes: self.lanes.clone(),
            signal: self.signal.clone(),
            gate: Arc::clone(&self.gate),
            settings: Arc::clone(&self.watch),
            clock: clock.clone(),
            max_items,
        });
        let name = TaskName::scoped("store", format!("{}/{}", self.gateway.cluster(), kind.info().name));
        let restart = Restart::Always(self.parts.settings.restart);
        let task = self.tasks.start(name, restart, move || watch::run(Arc::clone(&parts)));
        self.stores.push(Running { kind, store: Box::new(Store { kind, publisher, clock }), _task: task });
    }
}
