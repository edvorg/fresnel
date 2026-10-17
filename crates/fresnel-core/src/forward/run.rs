//! The task of one forward: it binds the listener, accepts the local connections, follows the
//! stores and the path of its cluster, and publishes the state.
//!
//! The listener outlives a run: after a panic, the next run takes it, so the local port stays
//! open and an automatic port does not change. The connections of a run end with it.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use fresnel_net::local::LocalListener;
use fresnel_net::{Cause, WallClock};
use tokio::sync::watch;
use tokio::task::{JoinError, JoinSet};
use tokio::time::{Instant, MissedTickBehavior, interval};

use super::accept::Accepts;
use super::conn;
use super::feed::{Feed, FeedChanges, StoreFacts};
use super::resolve::{Inputs, Owner, Resolution, resolve};
use super::rule::ForwardRule;
use super::shared::{Gate, Shared};
use super::state::{Facts, ForwardState, Listener, Path};
use super::view::{ForwardView, TargetData};
use crate::error::CoreError;
use crate::kinds::Kind;
use crate::published::Publisher;

/// The time between two looks at the counters and the times, while nothing else changes.
const TICK: Duration = Duration::from_secs(1);
/// The shortest time between two retries of the cluster that the forward asks for.
const RETRY_CALM: Duration = Duration::from_secs(10);

/// The feed of the cluster of a forward, or why it has none now.
pub(super) type FeedState = Result<Arc<Feed>, String>;

/// What each run of the task of a forward shares.
pub(super) struct Parts {
    pub(super) rule: ForwardRule,
    pub(super) shared: Arc<Shared>,
    pub(super) publisher: Publisher<ForwardView>,
    pub(super) feeds: watch::Receiver<FeedState>,
    /// Counts the "retry now" of the user: the cluster tests its path, and the failures in
    /// a row end.
    pub(super) retries: watch::Receiver<u64>,
    /// The open local connections at most.
    pub(super) limit: usize,
    /// The listener of the first run that bound it.
    pub(super) listener: Mutex<Option<Arc<LocalListener>>>,
    pub(super) clock: WallClock,
}

/// One run of the task: it ends only with a fault.
pub(super) async fn run(parts: Arc<Parts>) -> Result<(), CoreError> {
    let mut run = Run::new(&parts);
    let listener = match listener(&parts).await {
        Ok(listener) => listener,
        Err(reason) => {
            run.facts.listener = Listener::Failed(reason);
            run.publish(None);
            // A fixed port never changes silently: the user starts the forward again.
            return std::future::pending().await;
        }
    };
    run.facts.listener = Listener::Bound(listener.addresses().to_vec());
    let mut feeds = parts.feeds.clone();
    let mut feed = feeds.borrow_and_update().clone();
    let mut changes = feed.as_ref().ok().map(|feed| feed.changes());
    // The warm set of the cluster keeps upgrade connections ready for the next tunnels.
    let mut _warm = feed.as_ref().ok().map(|feed| feed.net.keep_upgrades_warm());
    let mut connections = JoinSet::new();
    let mut accepts = Accepts::new(parts.limit);
    let mut retries = parts.retries.clone();
    let mut tick = interval(TICK);
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        run.step(feed.as_ref());
        accepts.note(&parts.rule, &parts.shared.journal);
        tokio::select! {
            accepted = accepts.next(&listener) => {
                // A connection that ended leaves the count before the check of the limit.
                while let Some(done) = connections.try_join_next() {
                    joined(&parts, done);
                }
                if let Some(local) = accepts.take(accepted, connections.len(), &parts.shared.counts) {
                    connections.spawn(conn::serve(local, Arc::clone(&parts.shared), run.gate.subscribe()));
                }
            }
            Some(done) = connections.join_next(), if !connections.is_empty() => joined(&parts, done),
            () = parts.shared.changed.notified() => {}
            // An error: the owner of the forward is gone, and its drop stops this task next.
            Ok(()) = feeds.changed() => {
                feed = feeds.borrow_and_update().clone();
                changes = feed.as_ref().ok().map(|feed| feed.changes());
                _warm = feed.as_ref().ok().map(|feed| feed.net.keep_upgrades_warm());
            }
            () = next(&mut changes) => {}
            Ok(()) = retries.changed() => run.retry_now(feed.as_ref()),
            _ = tick.tick() => {}
        }
    }
}

/// Takes the end of the task of a local connection. It ends with no result; its outcome went
/// to the reports.
fn joined(parts: &Parts, done: Result<(), JoinError>) {
    if let Err(error) = done
        && error.is_panic()
    {
        let text = format!("{}: the task of a local connection panicked: {error}", parts.rule);
        parts.shared.journal.error(parts.rule.cluster.as_str(), text);
    }
}

/// The listener of the forward: the one of an earlier run, or a new bind.
async fn listener(parts: &Parts) -> Result<Arc<LocalListener>, String> {
    let kept = parts.listener.lock().unwrap_or_else(PoisonError::into_inner).clone();
    if let Some(listener) = kept {
        return Ok(listener);
    }
    let rule = &parts.rule;
    let bound = LocalListener::bind(&rule.bind.addresses(), rule.local.number()).await;
    let listener = Arc::new(bound.map_err(|error| error.to_string())?);
    *parts.listener.lock().unwrap_or_else(PoisonError::into_inner) = Some(Arc::clone(&listener));
    Ok(listener)
}

/// Waits for a change of the cluster. With no cluster, only the feed changes.
async fn next(changes: &mut Option<FeedChanges>) {
    match changes {
        Some(changes) => changes.next().await,
        None => std::future::pending().await,
    }
}

/// The state of one run.
struct Run<'a> {
    parts: &'a Parts,
    facts: Facts,
    gate: watch::Sender<Gate>,
    /// The state of the last publish, and the time of its step.
    shown: Option<(ForwardState, std::time::SystemTime)>,
    data: TargetData,
    /// The last retry of the cluster that the forward asked for.
    last_retry: Option<Instant>,
}

impl<'a> Run<'a> {
    fn new(parts: &'a Parts) -> Self {
        Self {
            parts,
            facts: Facts::new(),
            gate: watch::Sender::new(Gate::default()),
            shown: None,
            data: TargetData::NotLoaded,
            last_retry: None,
        }
    }

    /// Reads the facts of now, opens or closes the gate and publishes a new view.
    fn step(&mut self, feed: Result<&Arc<Feed>, &String>) {
        let (suspects, retry) = self.read_reports();
        match feed {
            Err(reason) => self.facts.path = Err(reason.clone()),
            Ok(feed) => {
                let status = feed.net.status();
                let path = Path::of(&status, crate::app::link_reason(&status), self.facts.was_up);
                self.facts.was_up |= path == Path::Up;
                if retry && path != Path::First {
                    self.retry(feed, false);
                }
                self.facts.path = Ok(path);
                self.resolve(feed, &suspects);
            }
        }
        let target = match (&self.facts.resolution, feed) {
            (Some(Resolution::Pod(pod)), Ok(feed)) if self.facts.opens() => Some((Arc::clone(feed), pod.clone())),
            _ => None,
        };
        self.gate.send_if_modified(|gate| {
            let same = match (&gate.target, &target) {
                (Some((old_feed, old)), Some((new_feed, new))) => Arc::ptr_eq(old_feed, new_feed) && old == new,
                (None, None) => true,
                _ => false,
            };
            if !same {
                gate.target.clone_from(&target);
            }
            !same
        });
        self.publish(feed.ok());
    }

    /// Takes the outcomes of the connections. Gives the pods that wait after the server did
    /// not find them, and true when a connection asks for a retry of the cluster.
    fn read_reports(&mut self) -> (Vec<Arc<str>>, bool) {
        let mut reports = self.parts.shared.reports();
        reports.expire(Instant::now());
        self.facts.failures = (reports.failures, reports.last_error.as_ref().map(|(_, text)| text.clone()));
        self.facts.no_tunnel = reports.no_tunnel.as_ref().map(|(_, reason)| reason.clone());
        self.facts.refused.clone_from(&reports.refused);
        let suspects = reports.suspects.iter().map(|(uid, _)| Arc::clone(uid)).collect();
        (suspects, std::mem::take(&mut reports.retry))
    }

    fn resolve(&mut self, feed: &Feed, suspects: &[Arc<str>]) {
        let rule = &self.parts.rule;
        let (pods, services, owners) = (feed.pods.read(), feed.services.read(), feed.owners.read());
        let mut stores = vec![StoreFacts::of(Kind::Pod, &pods)];
        let owner = match feed.owner(rule.kind, &rule.key) {
            Some((owner, facts)) => {
                stores.push(facts);
                owner
            }
            None => Owner::NotLoaded,
        };
        let behind: &[Kind] = match rule.kind {
            Kind::Service => {
                stores.push(StoreFacts::of(Kind::Service, &services));
                &[Kind::EndpointSlice]
            }
            Kind::Deployment => &[Kind::ReplicaSet],
            _ => &[],
        };
        let sources = owners.sources().iter().filter(|source| behind.contains(&source.kind));
        stores.extend(sources.map(|source| StoreFacts {
            kind: source.kind,
            listed: source.listed,
            state: source.state.clone(),
            since: source.since,
        }));
        let keep = match &self.facts.resolution {
            Some(Resolution::Pod(pod)) => Some(pod.clone()),
            _ => None,
        };
        let inputs = Inputs { pods: &pods, services: &services, owners: &owners, owner };
        self.facts.resolution = Some(resolve(rule, &inputs, keep.as_ref(), suspects));
        self.data = data(&stores);
    }

    /// Ends the waits of the channels of the cluster, which then test their connections and
    /// connect now: a local connection waits, or a tunnel broke. `always` for "retry now".
    fn retry(&mut self, feed: &Feed, always: bool) {
        let calm = self.last_retry.is_none_or(|last| last.elapsed() >= RETRY_CALM);
        if always || calm {
            self.last_retry = Some(Instant::now());
            feed.net.signal().raise(Cause::RetryNow);
        }
    }

    /// "Retry now" of the user: the failures in a row end, and the cluster tests its path.
    fn retry_now(&mut self, feed: Result<&Arc<Feed>, &String>) {
        {
            let mut reports = self.parts.shared.reports();
            reports.clear();
            reports.suspects.clear();
        }
        if let Ok(feed) = feed {
            self.retry(feed, true);
        }
        let text = format!("{}: retry now", self.parts.rule);
        self.parts.shared.journal.info(self.parts.rule.cluster.as_str(), text);
    }

    /// Publishes the view of now, and gives each new step of the state to the journal.
    fn publish(&mut self, feed: Option<&Arc<Feed>>) {
        let (parts, now) = (self.parts, self.parts.clock.now());
        let state = self.facts.state();
        let since = match &self.shown {
            Some((shown, since)) if shown.same_step(&state) => *since,
            _ => {
                self.journal(&state);
                now
            }
        };
        self.shown = Some((state.clone(), since));
        let (connections, bytes) = parts.shared.counts.read();
        let last_error = parts.shared.reports().last_error.clone();
        let addresses = match &self.facts.listener {
            Listener::Bound(addresses) => addresses.clone(),
            Listener::Binding | Listener::Failed(_) => Vec::new(),
        };
        let target = match (&self.facts.resolution, feed) {
            (Some(Resolution::Pod(pod)), Some(_)) => Some(pod.clone()),
            _ => None,
        };
        let view = ForwardView {
            rule: parts.rule.clone(),
            state,
            since,
            addresses,
            target,
            data: self.data.clone(),
            last_error,
            connections,
            bytes,
        };
        if *parts.publisher.read() != view {
            parts.publisher.publish(view);
        }
    }

    fn journal(&self, state: &ForwardState) {
        let (journal, rule) = (&self.parts.shared.journal, &self.parts.rule);
        let text = format!("{rule}: {state}");
        match state {
            ForwardState::Degraded(_) | ForwardState::Reconnecting { .. } | ForwardState::Failed(_) => {
                journal.warning(rule.cluster.as_str(), text);
            }
            ForwardState::Stopped | ForwardState::Starting(_) | ForwardState::Active => {
                journal.info(rule.cluster.as_str(), text);
            }
        }
    }
}

/// How fresh the stores behind the target are.
fn data(stores: &[StoreFacts]) -> TargetData {
    if stores.iter().any(|store| !store.listed) {
        return TargetData::NotLoaded;
    }
    match stores.iter().find(|store| !store.state.is_live()) {
        Some(store) => TargetData::Old { kind: store.kind, since: store.since },
        None => TargetData::Current,
    }
}
