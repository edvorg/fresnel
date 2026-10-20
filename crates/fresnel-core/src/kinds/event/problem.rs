//! The state now of the problem of a warning Event (plan section 5, rule 4: no old data
//! that looks current).
//!
//! An Event tells the last report of a problem, not its end: the cluster keeps it for about
//! one hour after that report (`--event-ttl`), and the kubelet reports a problem that stays
//! only some times in a few minutes. So the time of an Event cannot tell if its problem is
//! still there. The live object can: a rule of the kind and the reason of the Event reads
//! it in the stores of the cluster (`rules`). With no rule, or with a store that is not
//! live, the state is "unknown" with the reason, never a guess. When the live object proves
//! neither, the pace of the reports of the kubelet can tell that the problem most likely
//! ended (`quiet`).

mod container;
mod quiet;
mod rules;
mod workload;

use std::time::SystemTime;

use k8s_openapi::api::core::v1::Event;
use k8s_openapi::jiff::Timestamp;

use crate::index::{Index, ObjectEvents, last_seen};
use crate::kinds::{Describe, Kind, Tier, Visit};
use crate::published::Held;
use crate::search::Source;
use crate::store::{ObjectKey, Snapshot, StoreStatus};

pub use quiet::{QUIET_AFTER, REPORT_PACE};

/// What the app knows now of the problem of a warning Event. The sequence is the sequence
/// of a sort: the problems that need the user first, an unknown one among them, then the
/// quiet ones, which most likely ended, then the ended ones, each end by its time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProblemState {
    /// The object is not healthy now in the way that the reason of the Event names.
    Ongoing,
    /// The app cannot tell: no rule fits the Event, the store of the object is not live, or
    /// the live object proves neither and the reports give no evidence yet.
    Unknown,
    /// The live object proves neither, and since the last report, at this time, the kubelet
    /// sent none for longer than it takes to report a problem that goes on again (`quiet`):
    /// evidence of an end, not a proof.
    Quiet(Timestamp),
    /// The object is healthy now, and that began after the last report: at this time, when
    /// its status gives one.
    Resolved(Option<Timestamp>),
    /// An object with an other UID has the name now: the problem can go on in it.
    Replaced,
    /// The object is not in its live store.
    Deleted,
}

impl ProblemState {
    /// The place of the state in a short list of warnings, as the sequence of the type
    /// without the time of an end: a newer report of an end comes first.
    pub fn rank(self) -> u8 {
        match self {
            Self::Ongoing => 0,
            Self::Unknown => 1,
            Self::Quiet(_) => 2,
            Self::Resolved(_) => 3,
            Self::Replaced => 4,
            Self::Deleted => 5,
        }
    }
}

/// The state of the problem of a warning Event, with the fact that gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProblemNow {
    pub state: ProblemState,
    /// The fact in words, with no time: a view writes a time in the form of the user.
    pub why: String,
    /// True for an unknown state of a live object that proves neither that the problem goes
    /// on nor that it ended: the pace of the reports can tell more (`quiet`).
    undecided: bool,
}

impl ProblemNow {
    fn of(state: ProblemState, why: impl Into<String>) -> Self {
        Self { state, why: why.into(), undecided: false }
    }

    fn ongoing(why: impl Into<String>) -> Self {
        Self::of(ProblemState::Ongoing, why)
    }

    fn resolved(since: Option<Timestamp>, why: impl Into<String>) -> Self {
        Self::of(ProblemState::Resolved(since), why)
    }

    fn unknown(why: impl Into<String>) -> Self {
        Self::of(ProblemState::Unknown, why)
    }

    /// "Unknown" for a live object that proves neither an end nor that the problem goes on.
    fn undecided(why: impl Into<String>) -> Self {
        Self { undecided: true, ..Self::unknown(why) }
    }

    /// The same state with no evidence of the reports: their pace does not hold for it.
    fn settled(self) -> Self {
        Self { undecided: false, ..self }
    }

    fn replaced(why: impl Into<String>) -> Self {
        Self::of(ProblemState::Replaced, why)
    }

    fn deleted(why: impl Into<String>) -> Self {
        Self::of(ProblemState::Deleted, why)
    }
}

/// The object of an Event as a rule reads it.
struct Subject<'a> {
    kind: Kind,
    key: ObjectKey,
    /// The UID that the Event names. `None` when its source wrote none of the API: the
    /// kubelet writes the name of its node there.
    uid: Option<&'a str>,
    /// The time of the last report.
    last: Option<Timestamp>,
    /// The part of the object, for example `spec.containers{web}`.
    part: Option<&'a str>,
    /// The message, which names the probe of a failed one.
    message: &'a str,
}

/// The stores of the full objects of one cluster at one moment, as the rules read them: each
/// store of a kind of tier A besides the Events, and the Event index for the pace of the
/// reports. A view holds a copy for its computation.
#[derive(Clone)]
pub struct Witness {
    sources: Vec<Source>,
    /// The moment of the judgement: a container that runs again proves an end only after a
    /// time of run (`container`), and a report is old only after a time (`quiet`).
    at: SystemTime,
    /// The other reports of the object of an Event (`quiet`). `None` with no store of the
    /// Events: the state then needs no reports.
    reports: Option<Reports>,
}

/// The Events of a cluster as the pace of the reports reads them.
#[derive(Clone)]
struct Reports {
    index: Held<Index<ObjectEvents>>,
    /// The Events that left their store at its limit: the oldest by creation leave first, so
    /// a newer report of an older Event can be one of them.
    dropped: u64,
}

impl Witness {
    /// The witness of these stores at the moment `at`. The first store of a kind counts; a
    /// store of an other kind or of the metadata only does not.
    pub fn new(sources: impl IntoIterator<Item = Source>, at: SystemTime) -> Self {
        let mut kept: Vec<Source> = Vec::new();
        for source in sources {
            let kind = source.status.kind;
            if follows(kind) && kind.visit(IsFull(&source)) && kept.iter().all(|it| it.status.kind != kind) {
                kept.push(source);
            }
        }
        Self { sources: kept, at, reports: None }
    }

    /// The same witness with the Event index and the facts of the store of the Events.
    pub fn with_reports(self, index: Held<Index<ObjectEvents>>, store: &StoreStatus) -> Self {
        Self { reports: Some(Reports { index, dropped: store.dropped }), ..self }
    }

    /// True when the two hold the same snapshots at the same moment: a state that they give
    /// is the same.
    pub fn is_same(&self, other: &Self) -> bool {
        let same_reports = match (&self.reports, &other.reports) {
            (Some(a), Some(b)) => std::ptr::eq::<Index<ObjectEvents>>(&*a.index, &*b.index) && a.dropped == b.dropped,
            (a, b) => a.is_none() && b.is_none(),
        };
        self.at == other.at
            && same_reports
            && self.sources.len() == other.sources.len()
            && self
                .sources
                .iter()
                .zip(&other.sources)
                .all(|(a, b)| a.status.kind == b.status.kind && a.status.kind.visit(SameSnapshot(a, b)))
    }

    /// The state of the problem of an Event. `None` for an Event that is not a warning: it
    /// tells no problem.
    pub fn judge(&self, event: &Event) -> Option<ProblemNow> {
        if event.type_.as_deref() != Some("Warning") {
            return None;
        }
        let object = &event.involved_object;
        let reason = event.reason.as_deref().unwrap_or_default();
        let named = object.kind.as_deref().unwrap_or_default();
        let version = object.api_version.as_deref();
        let Some(kind) = Kind::named(named).filter(|kind| follows(*kind)) else {
            return Some(rules::no_rule(reason, named));
        };
        if !kind.fits_api(version) {
            let version = version.unwrap_or_default();
            return Some(ProblemNow::unknown(format!("the app has no rule for a {named} of {version}")));
        }
        let name = object.name.as_deref().unwrap_or_default();
        let namespace = object.namespace.as_deref().or(event.metadata.namespace.as_deref()).unwrap_or_default();
        let namespace = if kind.info().namespaced { namespace } else { "" };
        let uid = object.uid.as_deref().filter(|uid| !uid.is_empty() && *uid != name);
        let part = object.field_path.as_deref();
        let message = event.message.as_deref().unwrap_or_default();
        let key = ObjectKey::new(namespace, name);
        let subject = Subject { kind, key, uid, last: last_seen(event), part, message };
        Some(quiet::judge(rules::judge(self, reason, &subject), event, &subject, self))
    }

    /// The object of the Event in its store. The reason as a state when the store cannot
    /// give it: "unknown" for a store that is not live, "deleted" for an object that is not
    /// there, "replaced" for an other object with its name.
    fn find<K: Describe>(&self, key: &ObjectKey, uid: Option<&str>) -> Result<&K, ProblemNow> {
        let snapshot = self.store::<K>()?;
        match snapshot.items.get(key) {
            None => Err(ProblemNow::deleted(format!("the {} is not in its store", K::KIND))),
            Some(object) if uid.is_some_and(|uid| object.metadata().uid.as_deref() != Some(uid)) => {
                Err(ProblemNow::replaced(format!("an other {} has its name now", K::KIND)))
            }
            Some(object) => Ok(object),
        }
    }

    /// The live store of a kind.
    fn store<K: Describe>(&self) -> Result<&Snapshot<K>, ProblemNow> {
        let source = self.sources.iter().find(|source| source.status.kind == K::ID);
        let absent = || ProblemNow::unknown(format!("the cluster has no store of the {}", K::TITLE));
        let snapshot: &Snapshot<K> = source.and_then(Source::snapshot::<K>).ok_or_else(absent)?;
        if !snapshot.state.is_live() {
            return Err(ProblemNow::unknown(format!("the store of the {} is not live: {}", K::TITLE, snapshot.state)));
        }
        Ok(snapshot)
    }
}

/// True for a kind whose store of full objects the witness holds.
fn follows(kind: Kind) -> bool {
    kind.info().tier == Tier::A && kind != Kind::Event
}

/// True for a source of the full objects of its kind.
struct IsFull<'a>(&'a Source);

impl Visit for IsFull<'_> {
    type Output = bool;

    fn visit<K: Describe>(self) -> bool {
        self.0.snapshot::<K>().is_some()
    }
}

/// True when two sources of one kind hold one snapshot.
struct SameSnapshot<'a>(&'a Source, &'a Source);

impl Visit for SameSnapshot<'_> {
    type Output = bool;

    fn visit<K: Describe>(self) -> bool {
        match (self.0.snapshot::<K>(), self.1.snapshot::<K>()) {
            (Some(a), Some(b)) => std::ptr::eq::<Snapshot<K>>(&**a, &**b),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests;
