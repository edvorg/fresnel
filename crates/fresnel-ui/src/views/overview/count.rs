//! What the worker of an overview counts in one cluster (plan section 7.3): the objects of
//! each kind by health, the pods by phase, the nodes, and the warning Events of the last hour
//! with the state of each problem now. A count reads the snapshots of one request, so the
//! counts of a cluster are of one moment. The namespace filter holds for each kind with a
//! namespace, as in a table: the table that a link opens then has the rows of the count.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use fresnel_core::app::Objects;
use fresnel_core::cluster::ClusterId;
use fresnel_core::index::last_seen;
use fresnel_core::kinds::{Describe, Kind, ObjectHealth, ProblemNow, ProblemState, Sidebar, Visit, Witness};
use fresnel_core::metrics::{Amount, Metrics};
use fresnel_core::published::Held;
use fresnel_core::store::{ObjectKey, Snapshot};
use k8s_openapi::api::core::v1::{Event, Node, Pod};

use crate::views::metrics::gauge::Resource;
use crate::widgets::worker::Cancel;

/// The most warning Events that a block lists: the ongoing ones, the unknown ones, the quiet
/// ones, then the ended ones, the newest first in each part.
const NEWEST: usize = 8;
/// The warnings whose last report is in this time before now count. The cluster keeps an
/// Event for about one hour after its last report (`--event-ttl`); some keep it longer.
const HOUR: Duration = Duration::from_secs(3600);
/// The start of that hour moves in steps of this many seconds, so the counts follow the time
/// with no new count in each frame.
const HOUR_STEP: u64 = 10;

/// The phases of a pod, in the sequence of a block. A pod with no phase is `Unknown`.
const PHASES: [&str; 5] = ["Running", "Pending", "Succeeded", "Failed", "Unknown"];

/// The objects of one cluster that a request reads: the snapshots of one frame.
#[derive(Clone)]
pub(super) struct Input {
    pub(super) cluster: ClusterId,
    pub(super) pods: Held<Snapshot<Pod>>,
    /// The kinds of the overview that the core holds (`Command::ShowKinds`).
    pub(super) objects: Objects,
    /// The requests on each node at the last poll.
    pub(super) metrics: Held<Metrics>,
    /// The stores that tell the state of the problem of each warning now.
    pub(super) witness: Witness,
}

impl Input {
    /// True when the two hold the same snapshots and the same poll of the same cluster: the
    /// counts need no new run. The inputs of the last request hold their snapshots, so no
    /// new snapshot can have the address of an old one.
    pub(super) fn is_same(&self, other: &Self) -> bool {
        let kinds = |input: &Self| input.objects.kinds().collect::<Vec<Kind>>();
        self.cluster == other.cluster
            && std::ptr::eq::<Snapshot<Pod>>(&*self.pods, &*other.pods)
            && std::ptr::eq::<Metrics>(&*self.metrics, &*other.metrics)
            && self.witness.is_same(&other.witness)
            && kinds(self) == kinds(other)
            && kinds(self).into_iter().all(|kind| kind.visit(Same(&self.objects, &other.objects)))
    }
}

/// True when two sets of objects hold the same snapshot of a kind.
struct Same<'a>(&'a Objects, &'a Objects);

impl Visit for Same<'_> {
    type Output = bool;

    fn visit<K: Describe>(self) -> bool {
        match (self.0.get::<K>(), self.1.get::<K>()) {
            (Some(a), Some(b)) => std::ptr::eq::<Snapshot<K>>(&**a, &**b),
            (a, b) => a.is_none() && b.is_none(),
        }
    }
}

/// The objects of one kind by health.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Tally {
    healthy: u32,
    progressing: u32,
    warning: u32,
    failed: u32,
}

impl Tally {
    fn add(&mut self, health: ObjectHealth) {
        let count = match health {
            ObjectHealth::Healthy => &mut self.healthy,
            ObjectHealth::Progressing => &mut self.progressing,
            ObjectHealth::Warning => &mut self.warning,
            ObjectHealth::Failed => &mut self.failed,
        };
        *count += 1;
    }

    pub(super) fn of(self, health: ObjectHealth) -> u32 {
        match health {
            ObjectHealth::Healthy => self.healthy,
            ObjectHealth::Progressing => self.progressing,
            ObjectHealth::Warning => self.warning,
            ObjectHealth::Failed => self.failed,
        }
    }

    pub(super) fn total(self) -> u32 {
        self.healthy + self.progressing + self.warning + self.failed
    }

    /// The objects that the "problems" filter of a table shows.
    pub(super) fn problems(self) -> u32 {
        self.warning + self.failed
    }
}

/// The nodes of a cluster. The namespace filter does not hold for them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Nodes {
    pub(super) health: Tally,
    /// The nodes whose condition `Ready` is true.
    pub(super) ready: u32,
    /// The nodes that take no new pod.
    pub(super) cordoned: u32,
    /// The sum of the allocatable part of each node, of CPU and of memory. `None` when a
    /// node has no value: a sum of some nodes would give a false part.
    pub(super) allocatable: [Option<u64>; 2],
    /// The requests of the pods on the nodes at the last poll of the metrics.
    pub(super) requests: Amount,
}

/// One warning Event with the state of its problem now.
#[derive(Clone, Debug)]
pub(super) struct Warning {
    pub(super) event: Arc<Event>,
    pub(super) now: ProblemNow,
}

/// The warning Events of a cluster whose last report is in the last hour.
#[derive(Clone, Debug, Default)]
pub(super) struct Warnings {
    /// Each warning Event that the store keeps: the rows of the table of the link. A
    /// cluster can keep an Event longer than the hour.
    pub(super) all: u32,
    pub(super) count: u32,
    /// Of them, the ones whose problem is ongoing now, the ones whose state the app cannot
    /// tell, and the quiet ones, which most likely ended: "none ongoing" would be false while
    /// some are unknown, and a guess while some are quiet.
    pub(super) ongoing: u32,
    pub(super) unknown: u32,
    pub(super) quiet: u32,
    /// The ongoing ones first, then the unknown ones, the quiet ones and the ended ones, the
    /// newest first in each part: an unknown one can still be a problem.
    pub(super) shown: Vec<Warning>,
}

/// What the worker counted in one cluster.
#[derive(Clone, Debug)]
pub(super) struct Counted {
    pub(super) cluster: ClusterId,
    pub(super) pods: Tally,
    /// The pods of each phase, in the sequence of `PHASES`.
    pub(super) phases: Vec<(&'static str, u32)>,
    /// Each workload kind that the core holds, with its objects by health.
    pub(super) kinds: Vec<(Kind, Tally)>,
    /// `None` while the core holds no store of the nodes.
    pub(super) nodes: Option<Nodes>,
    /// `None` while the core holds no store of the Events.
    pub(super) warnings: Option<Warnings>,
}

/// The start of the last hour at `now`, in its steps: a warning reported after it counts.
/// The step goes up, so each warning that counts is of the last hour.
pub(super) fn hour_start(now: SystemTime) -> SystemTime {
    let seconds = now.duration_since(SystemTime::UNIX_EPOCH).map_or(0, |since| since.as_secs());
    let step = SystemTime::UNIX_EPOCH + Duration::from_secs(seconds.div_ceil(HOUR_STEP) * HOUR_STEP);
    step.checked_sub(HOUR).unwrap_or(SystemTime::UNIX_EPOCH)
}

/// Counts each cluster of a request, with the warnings reported after `since`. `None` when
/// a newer request waits.
pub(super) fn count(
    inputs: &[Input],
    namespaces: &BTreeSet<Arc<str>>,
    since: SystemTime,
    cancel: &Cancel<'_>,
) -> Option<Vec<Counted>> {
    inputs
        .iter()
        .map(|input| {
            if cancel.is_set() {
                return None;
            }
            let shows = |key: &ObjectKey| shows(key, namespaces);
            let (pods, phases) = pods(&input.pods, &shows);
            let workloads = input.objects.kinds().filter(|kind| kind.info().sidebar == Sidebar::Workloads);
            let kinds = workloads.filter_map(|kind| Some((kind, kind.visit(Tallied(input, &shows))?)));
            let nodes = input.objects.get::<Node>().map(|snapshot| self::nodes(snapshot, input));
            let warnings = input.objects.get::<Event>().map(|events| self::warnings(events, input, since, &shows));
            Some(Counted { cluster: input.cluster.clone(), pods, phases, kinds: kinds.collect(), nodes, warnings })
        })
        .collect()
}

/// True for an object that the namespace filter shows. An object with no namespace passes.
fn shows(key: &ObjectKey, namespaces: &BTreeSet<Arc<str>>) -> bool {
    namespaces.is_empty() || key.namespace.is_empty() || namespaces.contains(&key.namespace)
}

/// The objects of one kind by health. `None` for a kind that the core does not hold.
struct Tallied<'a, F>(&'a Input, &'a F);

impl<F: Fn(&ObjectKey) -> bool> Visit for Tallied<'_, F> {
    type Output = Option<Tally>;

    fn visit<K: Describe>(self) -> Option<Tally> {
        let snapshot = self.0.objects.get::<K>()?;
        let mut tally = Tally::default();
        snapshot.items.iter().filter(|(key, _)| (self.1)(key)).for_each(|(_, object)| tally.add(object.health()));
        Some(tally)
    }
}

/// The pods by health and by phase.
fn pods(snapshot: &Snapshot<Pod>, shows: &impl Fn(&ObjectKey) -> bool) -> (Tally, Vec<(&'static str, u32)>) {
    let (mut tally, mut phases) = (Tally::default(), PHASES.map(|phase| (phase, 0)));
    for (_, pod) in snapshot.items.iter().filter(|(key, _)| shows(key)) {
        tally.add(pod.health());
        let phase = pod.status.as_ref().and_then(|status| status.phase.as_deref()).unwrap_or("Unknown");
        let unknown = PHASES.len() - 1;
        let place = PHASES.iter().position(|known| *known == phase).unwrap_or(unknown);
        if let Some((_, count)) = phases.get_mut(place) {
            *count += 1;
        }
    }
    (tally, phases.to_vec())
}

fn nodes(snapshot: &Snapshot<Node>, input: &Input) -> Nodes {
    let mut nodes = Nodes { allocatable: [Some(0); 2], ..Nodes::default() };
    for (key, node) in snapshot.items.iter() {
        nodes.health.add(node.health());
        let status = node.status.as_ref();
        let conditions = status.and_then(|status| status.conditions.as_deref()).unwrap_or_default();
        nodes.ready += u32::from(conditions.iter().any(|it| it.type_ == "Ready" && it.status == "True"));
        nodes.cordoned += u32::from(node.spec.as_ref().and_then(|spec| spec.unschedulable) == Some(true));
        let allocatable = status.and_then(|status| status.allocatable.as_ref());
        for (sum, resource) in nodes.allocatable.iter_mut().zip(Resource::BOTH) {
            *sum = sum.zip(resource.read(allocatable)).map(|(sum, value)| sum.saturating_add(value));
        }
        let load = input.metrics.node_load(&input.cluster, &key.name);
        nodes.requests = nodes.requests + load.map(|load| load.requests).unwrap_or_default();
    }
    nodes
}

fn warnings(
    snapshot: &Snapshot<Event>,
    input: &Input,
    since: SystemTime,
    shows: &impl Fn(&ObjectKey) -> bool,
) -> Warnings {
    let recent = |event: &Event| last_seen(event).is_some_and(|last| SystemTime::from(last) >= since);
    let mut all = 0;
    let mut found: Vec<Warning> = snapshot
        .items
        .iter()
        .filter(|(key, event)| shows(key) && event.type_.as_deref() == Some("Warning"))
        .inspect(|_| all += 1)
        .filter(|(_, event)| recent(event))
        .filter_map(|(_, event)| Some(Warning { now: input.witness.judge(event)?, event: Arc::clone(event) }))
        .collect();
    let count_of = |state: fn(ProblemState) -> bool| found.iter().filter(|warning| state(warning.now.state)).count();
    let number = |count: usize| u32::try_from(count).unwrap_or(u32::MAX);
    let ongoing = number(count_of(|state| state == ProblemState::Ongoing));
    let unknown = number(count_of(|state| state == ProblemState::Unknown));
    let quiet = number(count_of(|state| matches!(state, ProblemState::Quiet(_))));
    let count = number(found.len());
    // A partial sort, as a cluster can keep 20,000 Events.
    let first = |a: &Warning, b: &Warning| {
        let rank = a.now.state.rank().cmp(&b.now.state.rank());
        rank.then_with(|| last_seen(&b.event).cmp(&last_seen(&a.event)))
    };
    if found.len() > NEWEST {
        found.select_nth_unstable_by(NEWEST, first);
        found.truncate(NEWEST);
    }
    found.sort_by(first);
    Warnings { all: number(all), count, ongoing, unknown, quiet, shown: found }
}

#[cfg(test)]
mod tests;
