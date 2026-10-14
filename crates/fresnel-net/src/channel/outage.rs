//! The outage of one cluster: the time from a connect that failed to the next one that works.
//!
//! A channel connects again after its own pause of a few seconds, with a full race. In a
//! long outage this costs much: in a break of 10 minutes, one cluster made 2,917 connect
//! attempts. Thus after the first part of an outage, the channels of a cluster connect in
//! turns: one attempt for each turn, one turn each few seconds for the whole cluster. A
//! signal of the cluster and the end of the outage end each wait at once.

use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime};

use tokio::sync::watch;
use tokio::time::Instant;

use crate::changes::Changed;
use crate::connect::Effort;
use crate::signal::{Listener, Signal};

/// The connects of a cluster in an outage. Each number is a first value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutageSettings {
    /// The first part of an outage. In it, each channel connects after its own pause.
    pub paced_after: Duration,
    /// The time between two turns of the cluster after the first part.
    pub turn: Duration,
    /// The part of `turn` that jitter adds or removes, in percent, from 0 to 100.
    pub jitter_percent: u8,
}

impl Default for OutageSettings {
    /// A turn that waits for a held attempt of 3 s leaves 3 s at most with no attempt in
    /// progress. Thus the end of an outage shows inside the recovery limit of 10 s.
    fn default() -> Self {
        Self { paced_after: Duration::from_secs(60), turn: Duration::from_secs(5), jitter_percent: 20 }
    }
}

impl OutageSettings {
    /// The time from one turn to the next. `sample` is a random number. The jitter keeps
    /// the clusters behind one network from turns at the same instant.
    fn spacing(&self, sample: u32) -> Duration {
        let jitter = u32::from(self.jitter_percent.min(100));
        let percent = 100 - jitter + sample % (2 * jitter + 1);
        self.turn.saturating_mul(percent) / 100
    }
}

/// A copy of the facts of the outage of one cluster.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutageStatus {
    /// The start of the first connect that failed.
    pub since: SystemTime,
    /// The time between two connect attempts of the cluster. `None` in the first part of
    /// the outage, in which each channel connects after its own pause.
    pub turn: Option<Duration>,
}

/// The outage of one cluster. Its channels share one value.
pub(crate) struct Outage {
    settings: OutageSettings,
    state: Mutex<State>,
    /// Counts the outages that ended. Each count ends each wait of a channel.
    ends: watch::Sender<u64>,
    changed: Changed,
}

struct State {
    /// The start of the first connect that failed after the last one that worked, in the
    /// clock of the runtime and in the wall clock. `None` with no outage.
    since: Option<(Instant, SystemTime)>,
    /// The last turn that a channel took. The next turn is one spacing later.
    last_turn: Option<Instant>,
    /// A raise ends each wait for a turn, so the turns of that time are void.
    signal: Listener,
}

impl State {
    fn is_paced(&self, settings: &OutageSettings) -> bool {
        self.since.is_some_and(|(start, _)| start.elapsed() >= settings.paced_after)
    }
}

impl Outage {
    pub(crate) fn new(settings: OutageSettings, signal: &Signal, changed: Changed) -> Self {
        let state = State { since: None, last_turn: None, signal: signal.listen() };
        Self { settings, state: Mutex::new(state), ends: watch::Sender::new(0), changed }
    }

    /// A connect that started at `started` failed. The first one starts the outage.
    pub(super) fn failed(&self, started: Instant) {
        let began = {
            let mut state = self.state();
            let began = state.since.is_none();
            if began {
                let wall = SystemTime::now().checked_sub(started.elapsed()).unwrap_or_else(SystemTime::now);
                state.since = Some((started, wall));
            }
            began
        };
        if began {
            self.changed.notify();
        }
    }

    /// A connect worked. The outage ends, and so does the wait of each other channel: the
    /// path to the server works again.
    pub(super) fn worked(&self) {
        let ended = {
            let mut state = self.state();
            state.last_turn = None;
            state.since.take().is_some()
        };
        if ended {
            self.ends.send_modify(|count| *count = count.wrapping_add(1));
            self.changed.notify();
        }
    }

    /// Waits before the next connect of a channel and gives the effort of that connect.
    /// `own` is the pause of the channel itself, for the first part of an outage. `show`
    /// gets each wait, for the state of the channel.
    pub(super) async fn pause(&self, listener: &mut Listener, own: Duration, show: impl Fn(Duration)) -> Effort {
        // The receiver comes first, so that an end after the checks below ends the wait.
        let mut ends = self.ends.subscribe();
        let paced = self.state().is_paced(&self.settings);
        if !paced {
            show(own);
            if woken(listener, &mut ends, own).await {
                return Effort::Race;
            }
        }
        // The first part can end during the pause of the channel.
        let Some(turn) = self.take_turn(fastrand::u32(..)) else {
            return Effort::Race;
        };
        let wait = turn.saturating_duration_since(Instant::now());
        show(wait);
        if woken(listener, &mut ends, wait).await { Effort::Race } else { Effort::Single }
    }

    /// The next free turn of the cluster for one channel. `None` outside the paced part.
    fn take_turn(&self, sample: u32) -> Option<Instant> {
        let mut state = self.state();
        if !state.is_paced(&self.settings) {
            return None;
        }
        let now = Instant::now();
        // A raise ended each wait, so no channel holds a turn that is still to come. The
        // races after the raise tested the path, so the next turn is one spacing later.
        if state.signal.was_raised() {
            state.last_turn = Some(now);
        }
        let turn = state.last_turn.map_or(now, |last| (last + self.settings.spacing(sample)).max(now));
        state.last_turn = Some(turn);
        Some(turn)
    }

    pub(crate) fn status(&self) -> Option<OutageStatus> {
        let state = self.state();
        let (_, since) = state.since?;
        Some(OutageStatus { since, turn: state.is_paced(&self.settings).then_some(self.settings.turn) })
    }

    /// Each use holds the lock for one small change or one copy, and never across a wait.
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Sleeps for `pause`. True when a signal or the end of the outage ended the sleep early.
async fn woken(listener: &mut Listener, ends: &mut watch::Receiver<u64>, pause: Duration) -> bool {
    tokio::select! {
        cause = listener.sleep(pause) => cause.is_some(),
        Ok(()) = ends.changed() => true,
    }
}

#[cfg(test)]
mod tests;
