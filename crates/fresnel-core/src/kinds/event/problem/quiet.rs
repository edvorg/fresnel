//! The pace of the reports of the kubelet, for a warning whose live object proves neither
//! that its problem goes on nor that it ended.
//!
//! The kubelet sends its Events through the recorder of client-go (`tools/record`). On k3s
//! it let the first 25 warnings of a pod through at once and then one each 5 minutes, and a
//! repeat of an Event raised the count and the last time of that Event. While a problem
//! goes on, the kubelet tries a report at each run of a probe and each retry of a sync or a
//! mount, so a report passes at least each `REPORT_PACE`: the longest measured gap of one
//! problem of a pod was 310 s. A warning with no report for twice the pace most likely
//! ended. That is evidence, not proof, so its state is "quiet", never "resolved".
//!
//! The warnings of one object share its turns: a failed liveness probe of a pod took each
//! turn of 15 minutes, while its readiness probe failed on with no report. So a newer
//! report of an other warning of the object keeps the state unknown. A report that the app
//! did not see proves nothing either: the evidence needs a current Event index and a store
//! of the Events that dropped none.
//!
//! A kubelet that is down, or that cannot reach the API server, sends no report while the
//! problem goes on, and its node turns not ready. So the silence counts only while the node
//! of the object is ready.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use k8s_openapi::api::core::v1::{Event, Node, Pod};
use k8s_openapi::jiff::Timestamp;

use super::rules::READY;
use super::{ProblemNow, ProblemState, Subject, Witness};
use crate::index::{Basis, last_seen};
use crate::kinds::Kind;
use crate::kinds::health::{Condition, condition};
use crate::store::ObjectKey;

/// The time in which the recorder of the kubelet lets one more warning of an object through
/// after its first 25: 300 s on k3s, from the first report on.
const REFILL: Duration = Duration::from_secs(300);
/// The longest wait of the kubelet between two tries of the report of a problem that goes
/// on, measured on k3s: a mount tries again after 122 s, a sync of a pod in a back-off after
/// 10 to 15 s, and a probe after its period. A probe with a longer period gets no evidence.
pub(super) const RETRY: Duration = Duration::from_secs(125);
/// The longest time between two reports of a problem that goes on: one turn of the recorder
/// and one retry, in whole minutes.
pub const REPORT_PACE: Duration = Duration::from_secs((REFILL.as_secs() + RETRY.as_secs()).div_ceil(60) * 60);
/// The time with no report after which an undecided warning of the kubelet is quiet: twice
/// the pace, a margin for a slow node or a late watch of the Events.
pub const QUIET_AFTER: Duration = Duration::from_secs(2 * REPORT_PACE.as_secs());
/// The component of the kubelet in an Event.
const KUBELET: &str = "kubelet";

/// "Quiet" for an undecided state of a warning of the kubelet with no report of its object
/// since `QUIET_AFTER`, while its node was ready; else the state of the rule.
pub(super) fn judge(now: ProblemNow, event: &Event, subject: &Subject<'_>, witness: &Witness) -> ProblemNow {
    let Some(last) = subject.last.filter(|_| now.undecided && of_kubelet(event)) else {
        return now;
    };
    let Some(ready) = ready_since(witness, subject) else {
        return now;
    };
    let from = SystemTime::from(last.max(ready));
    let silent = witness.at.duration_since(from).is_ok_and(|silent| silent >= QUIET_AFTER);
    if !silent || !alone(witness, subject, last) {
        return now;
    }
    // The fact of the object first: a row cuts the end of a long reason.
    let pace = REPORT_PACE.as_secs() / 60;
    let why = format!("{}; the kubelet repeats a problem that goes on at least each {pace} min", now.why);
    ProblemNow::of(ProblemState::Quiet(last), why)
}

/// The time since which the node whose kubelet reports about the object is ready: the node
/// of a pod, or the node itself. `None` when that node is not ready, is not in its live
/// store or gives no time: its kubelet may report nothing.
fn ready_since(witness: &Witness, subject: &Subject<'_>) -> Option<Timestamp> {
    let node = match subject.kind {
        Kind::Pod => {
            let pod = witness.find::<Pod>(&subject.key, subject.uid).ok()?;
            let name = pod.spec.as_ref()?.node_name.as_deref()?;
            witness.find::<Node>(&ObjectKey::new("", name), None).ok()?
        }
        Kind::Node => witness.find::<Node>(&subject.key, subject.uid).ok()?,
        _ => return None,
    };
    let conditions = node.status.as_ref()?.conditions.as_ref();
    condition(conditions, READY.name).filter(|ready| ready.status() == "True")?.since()
}

/// True for an Event of the kubelet. The recorder of the kubelet writes the source, and a
/// newer one the reporting component too.
fn of_kubelet(event: &Event) -> bool {
    let source = event.source.as_ref().and_then(|source| source.component.as_deref());
    source == Some(KUBELET) || event.reporting_component.as_deref() == Some(KUBELET)
}

/// True when no warning of the kubelet about the object, this one included, has a report
/// after `last` in a current Event index, and the store of the Events dropped none.
fn alone(witness: &Witness, subject: &Subject<'_>, last: Timestamp) -> bool {
    let Some(reports) = witness.reports.as_ref().filter(|reports| reports.dropped == 0) else {
        return false;
    };
    let answer = reports.index.events_of(subject.kind, &subject.key, subject.uid.unwrap_or_default());
    let warning = |event: &&Arc<Event>| event.type_.as_deref() == Some("Warning") && of_kubelet(event);
    answer.basis == Basis::Current
        && answer.found.iter().filter(warning).all(|event| last_seen(event).is_none_or(|seen| seen <= last))
}
