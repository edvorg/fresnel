//! The state that the handle, the driver and the connections of one proxy share.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::net::TcpListener;
use tokio::sync::watch;

use super::{ProxyCounters, ProxyMode};

/// A fixed seed gives the same sequence of holds in each run, so a test result can repeat.
const SEED: u64 = 0x5eed;

pub(super) struct Shared {
    pub(super) listener: TcpListener,
    pub(super) upstream: SocketAddr,
    pub(super) mode: watch::Sender<ProxyMode>,
    pub(super) counters: watch::Sender<ProxyCounters>,
    /// The number of the last reset that a test asked for.
    pub(super) resets_asked: watch::Sender<u64>,
    /// The number of the last reset that the driver completed.
    pub(super) resets_done: watch::Sender<u64>,
    /// Each connection with a smaller number is silent.
    silent_below: AtomicU64,
    random: Mutex<fastrand::Rng>,
}

/// What the driver decided for one new connection.
///
/// The connection counts as open until this value drops. The task of the connection owns
/// it, so the count is also correct after an abort or a panic of that task.
pub(super) struct Admission {
    /// The number of the connection in the sequence of accepts.
    pub(super) number: u64,
    /// The time with no answer before the proxy connects to the server.
    pub(super) wait: Duration,
    pub(super) shared: Arc<Shared>,
}

impl Drop for Admission {
    fn drop(&mut self) {
        self.shared.count(|counters| counters.open = counters.open.saturating_sub(1));
    }
}

impl Shared {
    pub(super) fn new(listener: TcpListener, upstream: SocketAddr) -> Self {
        Self {
            listener,
            upstream,
            mode: watch::Sender::new(ProxyMode::Pass),
            counters: watch::Sender::new(ProxyCounters::default()),
            resets_asked: watch::Sender::new(0),
            resets_done: watch::Sender::new(0),
            silent_below: AtomicU64::new(0),
            random: Mutex::new(fastrand::Rng::with_seed(SEED)),
        }
    }

    /// Counts a new connection as open and decides its wait from the mode.
    pub(super) fn admit(self: &Arc<Self>) -> Admission {
        let mode = *self.mode.borrow();
        let (wait, held) = match mode {
            ProxyMode::HoldNew { probability, min, max } => self.draw_hold(probability, min, max),
            // The SYN packet and its answer need one round trip.
            ProxyMode::Latency(one_way) => (one_way.saturating_mul(2), false),
            ProxyMode::Pass | ProxyMode::Freeze | ProxyMode::ResetEach(_) => (Duration::ZERO, false),
        };
        let mut number = 0;
        self.count(|counters| {
            number = counters.accepted;
            counters.accepted += 1;
            counters.open += 1;
            counters.held += u64::from(held);
        });
        Admission { number, wait, shared: Arc::clone(self) }
    }

    fn draw_hold(&self, probability: f64, min: Duration, max: Duration) -> (Duration, bool) {
        let mut random = self.random.lock().unwrap_or_else(PoisonError::into_inner);
        if random.f64() >= probability {
            return (Duration::ZERO, false);
        }
        (min.saturating_add(max.saturating_sub(min).mul_f64(random.f64())), true)
    }

    pub(super) fn count(&self, change: impl FnOnce(&mut ProxyCounters)) {
        self.counters.send_modify(change);
    }

    pub(super) fn silence_open(&self) {
        self.silent_below.store(self.counters.borrow().accepted, Ordering::SeqCst);
    }

    pub(super) fn is_silent(&self, number: u64) -> bool {
        number < self.silent_below.load(Ordering::SeqCst)
    }

    /// Asks the driver for a reset of all connections. Returns the number of the request.
    pub(super) fn request_reset(&self) -> u64 {
        let mut request = 0;
        self.resets_asked.send_modify(|asked| {
            *asked += 1;
            request = *asked;
        });
        request
    }
}
