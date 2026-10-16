//! The permissions of the user in one connected cluster (plan section 10): the rules of each
//! namespace that a view opens, and the exact answers of access reviews.
//!
//! One task of the cluster sends the reviews, one at a time. An answer stays for
//! `KEEP` and then a new request reads it again. A job that the server refuses with HTTP
//! 403 drops the rules of its namespace at once: the rules said yes, so they are old.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use fresnel_net::task::{Backoff, Restart, TaskHandle, TaskName, Tasks};
use tokio::sync::Notify;
use tokio::time::Instant;

use super::Inner;
use super::repaint::Repaint;
use crate::cluster::{ClusterId, Gateway};
use crate::error::CoreError;
use crate::ops::access::{self, Attributes, Decision, Rules};

/// How long an answer of the server stays. Research 08 section 11.3: "some minutes".
const KEEP: Duration = Duration::from_secs(300);
/// The most reviews that wait for the task. A view asks for few namespaces.
const QUEUE_LIMIT: usize = 64;
/// The most answers of each type that the core keeps: each view copies them in each frame.
/// The oldest answer leaves first, and a view that needs it asks again, so no count of the
/// ones that left is necessary. A review on its way stays; the queue limits those.
const KEPT_LIMIT: usize = 256;

/// How far the read of the rules of one namespace is.
#[derive(Clone, Debug, PartialEq)]
pub enum RulesState {
    /// The review is on its way.
    Reading,
    /// The rules of the server.
    Read(Arc<Rules>),
    /// The review failed.
    Failed(CoreError),
}

impl RulesState {
    fn is_reading(&self) -> bool {
        matches!(self, Self::Reading)
    }
}

/// How far one access review is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckState {
    /// The review is on its way.
    Asking,
    /// The answer of the server.
    Answered(Decision),
    /// The review failed.
    Failed(CoreError),
}

impl CheckState {
    fn is_asking(&self) -> bool {
        matches!(self, Self::Asking)
    }
}

/// What the views know of the permissions of the user in one cluster.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AccessView {
    /// The rules of each namespace that a view asked for.
    pub rules: BTreeMap<Arc<str>, RulesState>,
    /// The answer of each access review that a view asked for.
    pub checks: BTreeMap<Attributes, CheckState>,
}

impl AccessView {
    /// The decision for the requests of an action (`access::needs`): each one must be
    /// allowed. The answer of a review counts before the rules of the namespace.
    pub fn decide(&self, requests: &[Attributes]) -> Decision {
        Decision::all(requests.iter().map(|request| self.decide_one(request)))
    }

    fn decide_one(&self, request: &Attributes) -> Decision {
        if let Some(CheckState::Answered(decision)) = self.checks.get(request) {
            return decision.clone();
        }
        match self.rules.get(&request.namespace) {
            Some(RulesState::Read(rules)) => rules.decide(request),
            Some(RulesState::Failed(error)) => Decision::Unknown(format!("the read of the rules failed: {error}")),
            Some(RulesState::Reading) | None => Decision::Unknown("the rules are not read yet".to_owned()),
        }
    }
}

/// One review that waits for the task.
#[derive(Clone, Debug)]
enum Ask {
    Rules(Arc<str>),
    Check(Attributes),
}

#[derive(Default)]
struct Known {
    rules: BTreeMap<Arc<str>, (RulesState, Instant)>,
    checks: BTreeMap<Attributes, (CheckState, Instant)>,
    queue: VecDeque<Ask>,
}

#[derive(Default)]
struct Shared {
    known: Mutex<Known>,
    asked: Notify,
}

impl Shared {
    fn known(&self) -> MutexGuard<'_, Known> {
        self.known.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A handle that drops the rules of a namespace after a refusal of the server.
#[derive(Clone)]
pub(super) struct AccessHandle {
    shared: Arc<Shared>,
    repaint: Repaint,
}

impl AccessHandle {
    /// Drops the rules of a namespace and each answer about it. The next view that needs
    /// them asks again.
    pub(super) fn forget(&self, namespace: &str) {
        let mut known = self.shared.known();
        known.rules.retain(|of, _| &**of != namespace);
        known.checks.retain(|attributes, _| &*attributes.namespace != namespace);
        drop(known);
        self.repaint.request();
    }
}

/// The permissions of one connected cluster with their task. The task stops at the drop.
pub(super) struct ClusterAccess {
    handle: AccessHandle,
    _task: TaskHandle,
}

impl ClusterAccess {
    pub(super) fn start(tasks: &Tasks, cluster: &ClusterId, gateway: Gateway, repaint: Repaint) -> Self {
        let shared = Arc::new(Shared::default());
        let handle = AccessHandle { shared: Arc::clone(&shared), repaint: repaint.clone() };
        let name = TaskName::scoped("permissions", cluster.as_str());
        let task = tasks.start(name, Restart::Always(Backoff::default()), move || {
            serve(Arc::clone(&shared), gateway.clone(), repaint.clone())
        });
        Self { handle, _task: task }
    }

    /// The handle for the jobs of the cluster.
    pub(super) fn handle(&self) -> AccessHandle {
        self.handle.clone()
    }

    /// Asks for the rules of a namespace, unless an answer of the last `KEEP` exists or a
    /// review of them is on its way.
    pub(super) fn read_rules(&self, namespace: Arc<str>) {
        let mut known = self.handle.shared.known();
        if is_fresh(known.rules.get(&namespace), |state| matches!(state, RulesState::Failed(_))) {
            return;
        }
        self.ask(&mut known, Ask::Rules(namespace));
    }

    /// Asks the server if the user can make one request, unless an answer is fresh.
    pub(super) fn check(&self, attributes: Attributes) {
        let mut known = self.handle.shared.known();
        if is_fresh(known.checks.get(&attributes), |state| matches!(state, CheckState::Failed(_))) {
            return;
        }
        self.ask(&mut known, Ask::Check(attributes));
    }

    fn ask(&self, known: &mut Known, ask: Ask) {
        let now = Instant::now();
        let full = known.queue.len() >= QUEUE_LIMIT;
        let refused = || CoreError::invalid(format!("{QUEUE_LIMIT} reviews wait for the server"));
        match &ask {
            Ask::Rules(namespace) => {
                let state = if full { RulesState::Failed(refused()) } else { RulesState::Reading };
                keep(&mut known.rules, Arc::clone(namespace), (state, now), RulesState::is_reading);
            }
            Ask::Check(attributes) => {
                let state = if full { CheckState::Failed(refused()) } else { CheckState::Asking };
                keep(&mut known.checks, attributes.clone(), (state, now), CheckState::is_asking);
            }
        }
        if !full {
            known.queue.push_back(ask);
            self.handle.shared.asked.notify_one();
        }
        self.handle.repaint.request();
    }

    /// A copy for the view.
    pub(super) fn view(&self) -> Arc<AccessView> {
        let known = self.handle.shared.known();
        let rules = known.rules.iter().map(|(namespace, (state, _))| (Arc::clone(namespace), state.clone())).collect();
        let checks = known.checks.iter().map(|(attributes, (state, _))| (attributes.clone(), state.clone())).collect();
        Arc::new(AccessView { rules, checks })
    }
}

/// Puts an entry into a map of answers. At the limit, the oldest entry that is not on its
/// way (`waits`) leaves first.
fn keep<K: Ord + Clone, S>(
    map: &mut BTreeMap<K, (S, Instant)>,
    key: K,
    entry: (S, Instant),
    waits: impl Fn(&S) -> bool,
) {
    if map.len() >= KEPT_LIMIT && !map.contains_key(&key) {
        let answers = map.iter().filter(|(_, (state, _))| !waits(state));
        let oldest = answers.min_by_key(|(_, (_, at))| *at).map(|(key, _)| key.clone());
        if let Some(oldest) = oldest {
            map.remove(&oldest);
        }
    }
    map.insert(key, entry);
}

/// True for an answer of the last `KEEP`, or a review on its way. A failure is never fresh.
fn is_fresh<S>(entry: Option<&(S, Instant)>, failed: impl Fn(&S) -> bool) -> bool {
    entry.is_some_and(|(state, at)| !failed(state) && at.elapsed() < KEEP)
}

/// The body of the task: sends each review that waits, in its sequence.
async fn serve(shared: Arc<Shared>, gateway: Gateway, repaint: Repaint) -> Result<(), CoreError> {
    // A run before this one can have ended during a review, which then has no answer. A
    // review changes nothing, so each one with no answer goes again.
    {
        let mut known = shared.known();
        let rules = known.rules.iter().filter(|(_, (state, _))| state.is_reading());
        let mut queue: VecDeque<Ask> = rules.map(|(namespace, _)| Ask::Rules(Arc::clone(namespace))).collect();
        let checks = known.checks.iter().filter(|(_, (state, _))| state.is_asking());
        queue.extend(checks.map(|(attributes, _)| Ask::Check(attributes.clone())));
        known.queue = queue;
    }
    loop {
        let next = shared.known().queue.pop_front();
        let Some(ask) = next else {
            shared.asked.notified().await;
            continue;
        };
        match ask {
            Ask::Rules(namespace) => {
                let state = match access::read_rules(&gateway, &namespace).await {
                    Ok(rules) => RulesState::Read(Arc::new(rules)),
                    Err(error) => RulesState::Failed(error),
                };
                keep(&mut shared.known().rules, namespace, (state, Instant::now()), RulesState::is_reading);
            }
            Ask::Check(attributes) => {
                let state = match access::check(&gateway, &attributes).await {
                    Ok(decision) => CheckState::Answered(decision),
                    Err(error) => CheckState::Failed(error),
                };
                keep(&mut shared.known().checks, attributes, (state, Instant::now()), CheckState::is_asking);
            }
        }
        repaint.request();
    }
}

impl Inner {
    /// Reads the rules of the user in a namespace of a connected cluster.
    pub(super) fn read_rules(&self, cluster: &ClusterId, namespace: Arc<str>) {
        let state = self.state();
        if let Some(connected) = state.connected.get(cluster) {
            connected.access().read_rules(namespace);
        }
    }

    /// Asks a connected cluster if the user can make one request.
    pub(super) fn check_access(&self, cluster: &ClusterId, attributes: Attributes) {
        let state = self.state();
        if let Some(connected) = state.connected.get(cluster) {
            connected.access().check(attributes);
        }
    }
}

#[cfg(test)]
mod tests;
