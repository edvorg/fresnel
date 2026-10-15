//! The check of each store against the API (plan section 13, line 8).
//!
//! A task for each kind lists the kind on the path of the comparison each period and waits
//! until the store holds that state or a later one: the time to that point is the lag of the
//! store. A store that is behind for longer than the limit while it shows "live" is a gap
//! with no visible state, the fault that the soak looks for (design rule 4).

use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use fresnel_core::cluster::Gateway;
use fresnel_core::kinds::Kind;
use fresnel_core::net::task::{Backoff, Restart, TaskHandle, TaskName, Tasks};
use fresnel_core::store::StoreState;
use serde::Serialize;
use tokio::sync::watch;
use tokio::time::{Instant, sleep, sleep_until};

use super::api;
use super::cover::{Listed, Verdict, cover};
use super::look::Look;

/// The time from one check of a kind to the next one.
pub(super) const PERIOD: Duration = Duration::from_secs(10);
/// The most time that a store may show "live" while it is behind: the time in which a row of
/// a cluster with a silent network gets its mark (weakness 8 of plan section 6).
pub(super) const LIMIT: Duration = Duration::from_secs(20);
/// A check that takes this long is a gap of the report, with a visible state or not.
const GAP: Duration = Duration::from_secs(5);
/// A check ends after this time also when the store is still behind, so that a store that
/// never catches up is in the report.
const GIVE_UP: Duration = Duration::from_secs(300);
/// The most texts of states that a gap keeps: a state of a retry has its count of attempts in
/// its text, so a long gap has many.
const STATES: usize = 8;

/// One time in which a store was behind the API.
#[derive(Clone, Debug, Serialize)]
pub(super) struct Gap {
    pub(super) kind: &'static str,
    /// The seconds from the list to the first copy of the store that holds it.
    pub(super) seconds: f64,
    /// The longest time in seconds in which the store showed "live" and was behind.
    pub(super) live_seconds: f64,
    /// True when that time is longer than the limit: a gap with no visible state.
    pub(super) without_state: bool,
    /// The other states that the store showed.
    pub(super) states: Vec<String>,
    /// The first difference.
    pub(super) reason: String,
    /// True when the store was still behind when the check gave up.
    pub(super) open: bool,
}

/// What the checks found since the last minute of the report.
#[derive(Debug, Default)]
pub(super) struct Tally {
    pub(super) lags: Vec<Duration>,
    pub(super) gaps: Vec<Gap>,
    /// The lists of the comparison that failed, by their reason.
    pub(super) failed: BTreeMap<String, u64>,
}

/// The tally that the checks and the minutes share.
pub(super) type Shared = Arc<Mutex<Tally>>;

fn add(shared: &Shared, change: impl FnOnce(&mut Tally)) {
    change(&mut shared.lock().unwrap_or_else(PoisonError::into_inner));
}

/// Starts the check of each kind. The starts spread over one period. Each check stops when
/// its handle drops.
pub(super) fn start(
    tasks: &Tasks,
    kinds: &[Kind],
    gateway: &Gateway,
    looks: &watch::Receiver<Arc<Look>>,
    tally: &Shared,
) -> Vec<TaskHandle> {
    let count = u32::try_from(kinds.len()).unwrap_or(u32::MAX).max(1);
    let each = |(index, kind): (u32, &Kind)| {
        let (kind, offset) = (*kind, PERIOD * index / count);
        let (gateway, looks, tally) = (gateway.clone(), looks.clone(), Arc::clone(tally));
        let name = TaskName::scoped("soak check", kind.info().name);
        tasks.start(name, Restart::Always(Backoff::default()), move || {
            follow(kind, offset, gateway.clone(), looks.clone(), Arc::clone(&tally))
        })
    };
    (0..).zip(kinds).map(each).collect()
}

async fn follow(
    kind: Kind,
    offset: Duration,
    gateway: Gateway,
    mut looks: watch::Receiver<Arc<Look>>,
    tally: Shared,
) -> Result<(), Infallible> {
    sleep(offset).await;
    let mut next = Instant::now();
    loop {
        sleep_until(next).await;
        next = Instant::now() + PERIOD;
        let first = match api::list(&gateway, kind).await {
            Ok(first) => first,
            Err(error) => {
                add(&tally, |tally| *tally.failed.entry(format!("{}: {error}", kind.info().name)).or_default() += 1);
                continue;
            }
        };
        let (lag, gap) = until_covered(kind, &first, &gateway, &mut looks).await;
        add(&tally, |tally| {
            tally.lags.push(lag);
            tally.gaps.extend(gap);
        });
    }
}

/// What a check saw while the store was behind.
struct Behind {
    kind: &'static str,
    start: Instant,
    /// Since when the store shows "live" and is behind.
    live_since: Option<Instant>,
    longest_live: Duration,
    states: BTreeSet<String>,
    reason: String,
}

impl Behind {
    fn see(&mut self, state: Option<&StoreState>, at: Instant, reason: String) {
        let at = at.max(self.start);
        match state {
            Some(StoreState::Live) => {
                let since = *self.live_since.get_or_insert(at);
                self.longest_live = self.longest_live.max(at - since);
            }
            Some(other) => {
                self.live_since = None;
                self.note(other.to_string());
            }
            None => {
                self.live_since = None;
                self.note("the cluster has no stores".to_owned());
            }
        }
        if self.reason.is_empty() {
            self.reason = reason;
        }
    }

    fn note(&mut self, state: String) {
        if self.states.len() < STATES {
            self.states.insert(state);
        }
    }

    /// The lag of the check, and its gap when it was long.
    fn end(mut self, at: Instant, open: bool) -> (Duration, Option<Gap>) {
        let at = at.max(self.start);
        if let Some(since) = self.live_since {
            self.longest_live = self.longest_live.max(at - since);
        }
        let lag = at - self.start;
        let without_state = self.longest_live > LIMIT;
        if lag < GAP && !without_state && !open {
            return (lag, None);
        }
        let gap = Gap {
            kind: self.kind,
            seconds: lag.as_secs_f64(),
            live_seconds: self.longest_live.as_secs_f64(),
            without_state,
            states: self.states.into_iter().collect(),
            reason: self.reason,
            open,
        };
        (lag, Some(gap))
    }
}

/// Waits until a copy of the core has the state of the list `first` or a later one.
async fn until_covered(
    kind: Kind,
    first: &Listed,
    gateway: &Gateway,
    looks: &mut watch::Receiver<Arc<Look>>,
) -> (Duration, Option<Gap>) {
    let start = Instant::now();
    let mut behind = Behind {
        kind: kind.info().name,
        start,
        live_since: None,
        longest_live: Duration::ZERO,
        states: BTreeSet::new(),
        reason: String::new(),
    };
    loop {
        let look = Arc::clone(&looks.borrow_and_update());
        let store = look.store(kind);
        let verdict = match &store {
            None => Verdict::Behind("the cluster has no stores".to_owned()),
            Some(store) => {
                let partial = store.dropped > 0;
                match cover(first, &store.versions, None, partial) {
                    Verdict::NeedsLater => match api::list(gateway, kind).await {
                        Ok(later) => cover(first, &store.versions, Some(&later), partial),
                        Err(error) => Verdict::Behind(format!("the second list failed: {error}")),
                    },
                    verdict => verdict,
                }
            }
        };
        let reason = match verdict {
            Verdict::Covered => return behind.end(look.at, false),
            Verdict::NeedsLater => "the second list did not decide".to_owned(),
            Verdict::Behind(reason) => reason,
        };
        behind.see(store.as_ref().map(|store| &store.state), look.at, reason);
        if look.at - start > GIVE_UP {
            return behind.end(look.at, true);
        }
        // The sender lives as long as the soak, and the soak stops this task first.
        if looks.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use fresnel_core::store::Wait;

    use super::*;

    fn behind(start: Instant) -> Behind {
        let states = BTreeSet::new();
        Behind { kind: "Pod", start, live_since: None, longest_live: Duration::ZERO, states, reason: String::new() }
    }

    #[test]
    fn a_store_that_shows_live_and_is_behind_past_the_limit_is_a_gap_with_no_visible_state() {
        let start = Instant::now();
        let at = |seconds| start + Duration::from_secs(seconds);
        let mut check = behind(start);
        for second in [0, 10, 21] {
            check.see(Some(&StoreState::Live), at(second), "ns/a has an older version".to_owned());
        }
        let (lag, gap) = check.end(at(22), false);
        let gap = gap.expect("a gap");
        assert_eq!((lag, gap.kind, gap.without_state, gap.live_seconds), (at(22) - start, "Pod", true, 22.0));
        assert_eq!(gap.reason, "ns/a has an older version");
    }

    #[test]
    fn a_visible_state_ends_the_live_time_and_a_short_lag_is_no_gap() {
        let start = Instant::now();
        let at = |seconds| start + Duration::from_secs(seconds);
        let mut check = behind(start);
        let waits = StoreState::Waiting(Wait::Channel);
        check.see(Some(&StoreState::Live), at(0), "first".to_owned());
        check.see(Some(&StoreState::Live), at(14), "second".to_owned());
        check.see(Some(&waits), at(15), "third".to_owned());
        check.see(Some(&StoreState::Live), at(40), "fourth".to_owned());
        let gap = check.end(at(45), false).1.expect("a gap of 45 s");
        assert_eq!((gap.without_state, gap.live_seconds, gap.reason.as_str()), (false, 14.0, "first"));
        assert_eq!(gap.states, ["waits for a connection"]);

        let mut check = behind(start);
        check.see(Some(&StoreState::Live), at(0), "an event on its way".to_owned());
        let (lag, gap) = check.end(at(1), false);
        assert_eq!((lag, gap.is_none()), (Duration::from_secs(1), true));
    }

    #[test]
    fn a_gap_keeps_a_limited_count_of_states() {
        let start = Instant::now();
        let mut check = behind(start);
        for failures in 0..20 {
            let state = StoreState::Attempt {
                failures,
                reason: fresnel_core::CoreError::new(fresnel_core::net::ErrorClass::Network, "no connection"),
            };
            check.see(Some(&state), start, String::new());
        }
        assert_eq!(check.end(start + GIVE_UP, true).1.map(|gap| (gap.states.len(), gap.open)), Some((STATES, true)));
    }
}
