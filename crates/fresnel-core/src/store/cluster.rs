//! The stores of one cluster: one that is always on for each kind of tier A and tier B,
//! and the stores on demand.

use std::sync::Arc;
use std::time::Duration;

use fresnel_net::WallClock;
use fresnel_net::task::{Backoff, Restart, TaskHandle, TaskName, Tasks};
use tokio::sync::watch::Sender;

use super::demand::{Demand, Demands};
use super::snapshot::{Snapshot, StoreStatus};
use super::starter::{Form, Running, Starter};
use crate::cluster::{ClusterId, Link};
use crate::error::CoreError;
use crate::kinds::{Describe, Kind, Meta, Object, Tier, Visit};
use crate::published::{Bin, Changes, Notice, Reader};
use crate::search::Source;
use crate::watch::{self, LaneStates, Lanes, ListGate, WatchSettings};

/// Which Events a cluster keeps (plan section 9.6). Events can be most of the objects of
/// a cluster.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EventsMode {
    /// Each Event.
    #[default]
    All,
    /// Only the Events of the type `Warning`. The server selects them.
    Warnings,
    /// A store on demand only, while a view needs the Events.
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
    /// The time that a store on demand stays on after the last view that needed it closed.
    pub linger: Duration,
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
        Self {
            watch: WatchSettings::default(),
            events: EventsMode::default(),
            max_events: 20_000,
            restart,
            linger: Duration::from_secs(60),
        }
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

/// The stores of one cluster. Each one stops when this value drops.
pub struct ClusterStores {
    cluster: ClusterId,
    stores: Vec<Running>,
    demands: Demands,
    notice: Notice,
    /// The sender of the lanes, for the count of their readers in the memory account.
    lanes: Sender<LaneStates>,
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
                let (net, states) = (link.net.clone(), states.clone());
                move || watch::follow(net.clone(), states.clone())
            });
        let notice = Notice::new();
        let linger = parts.settings.linger;
        let starter = Starter {
            tasks: tasks.clone(),
            gateway: link.gateway.clone(),
            signal: link.net.signal(),
            lanes,
            gate: ListGate::new(parts.settings.watch.lists),
            watch: Arc::new(parts.settings.watch.clone()),
            parts,
            notice: notice.clone(),
        };
        let stores = Kind::ALL.iter().filter_map(|kind| kind.visit(AlwaysOn(&starter))).collect();
        let demands = Demands::start(starter, linger);
        Ok(Self { cluster, stores, demands, notice, lanes: states, _lanes: lanes_task })
    }

    /// The cluster of the stores.
    pub fn cluster(&self) -> &ClusterId {
        &self.cluster
    }

    /// Tells when a store of the cluster has a new snapshot: new objects or a new state.
    /// A store on demand tells it too.
    pub fn changes(&self) -> Changes {
        self.notice.changes()
    }

    /// The notice of each new snapshot of a store of the cluster, for a task that makes a
    /// reader of it at each run.
    pub(crate) fn notice(&self) -> Notice {
        self.notice.clone()
    }

    /// The facts of each store that is always on, in the sequence of the kinds.
    pub fn statuses(&self) -> Vec<StoreStatus> {
        self.stores.iter().map(Running::status).collect()
    }

    /// The snapshot of each store that is always on, in the sequence of the kinds, for the
    /// quick search. The call copies one pointer for each store.
    pub fn sources(&self) -> Vec<Source> {
        self.stores.iter().map(Running::source).collect()
    }

    /// The snapshot of each store on demand that runs, for the memory account: a store
    /// stays some time after its last holder, and no view shows it then.
    pub fn demand_sources(&self) -> Vec<Source> {
        self.demands.sources()
    }

    /// The readers of the lanes of the cluster: the starter of the stores and the loop of
    /// each store, for the memory account. A loop that ended and kept its reader shows here.
    pub fn lane_readers(&self) -> usize {
        self.lanes.receiver_count()
    }

    /// A reader of the store of a tier A kind, with its full objects. `None` for a kind of
    /// an other tier, and for the Events when the settings give them no store.
    pub fn full<K: Describe>(&self) -> Option<Reader<Snapshot<K>>> {
        self.reader(K::ID)
    }

    /// A reader of the store of a tier B kind, with the metadata of its objects. `None`
    /// for a kind of an other tier.
    pub fn metadata(&self, kind: Kind) -> Option<Reader<Snapshot<Meta>>> {
        self.reader(kind)
    }

    /// The full objects of a kind, for a view. A kind with a store of full objects that
    /// is always on gives that store. For each other kind, the first holder starts a store
    /// of its full objects, and that store stops when its time ends after the last holder
    /// went (plan section 9.5). The Events of the setting "on demand" have such a store,
    /// with the most Events of the settings. `None` only for a type that is not the type
    /// of its kind.
    pub fn demand<K: Describe>(&self) -> Option<Demand<K>> {
        if let Some(reader) = self.full::<K>() {
            return Some(Demand::always(reader));
        }
        let max_items = (K::ID == Kind::Event).then_some(self.demands.settings().max_events);
        self.demands.demand::<K>(max_items)
    }

    fn reader<T: Object>(&self, kind: Kind) -> Option<Reader<Snapshot<T>>> {
        self.stores.iter().find(|running| running.kind == kind)?.reader()
    }
}

impl std::fmt::Debug for ClusterStores {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let stores = self.stores.len();
        f.debug_struct("ClusterStores").field("cluster", &self.cluster).field("stores", &stores).finish()
    }
}

/// The field selector for the Events of the type `Warning`.
const WARNINGS: &str = "type=Warning";

/// Starts the store that is always on of one kind: the full objects of a tier A kind, and
/// the metadata of a tier B kind.
struct AlwaysOn<'a>(&'a Starter);

impl Visit for AlwaysOn<'_> {
    type Output = Option<Running>;

    fn visit<K: Describe>(self) -> Option<Running> {
        let settings = &self.0.parts.settings;
        match (K::TIER, K::ID, settings.events) {
            (Tier::C, ..) | (_, Kind::Event, EventsMode::OnDemand) => None,
            (Tier::B, ..) => Some(self.0.start::<Meta>(K::ID, Form::Metadata, None, None)),
            (Tier::A, Kind::Event, mode) => {
                let fields = (mode == EventsMode::Warnings).then_some(WARNINGS);
                Some(self.0.start::<K>(K::ID, Form::Full, fields, Some(settings.max_events)))
            }
            (Tier::A, ..) => Some(self.0.start::<K>(K::ID, Form::Full, None, None)),
        }
    }
}
