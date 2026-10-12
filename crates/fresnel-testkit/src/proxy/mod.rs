//! The fault proxy: a TCP proxy between a client and a server that makes network faults.
//!
//! It needs no root permission, and TLS passes through it. The kernel accepts each connection
//! and acknowledges each byte, so the proxy cannot drop a packet. A held connection is thus
//! open at the TCP level and gets no answer to its first bytes. A test of TCP keepalive needs
//! a different tool.
//!
//! The proxy has one mode at a time. To combine two faults, put one proxy before a second one.

mod drain;
mod driver;
mod pipe;
mod relay;
mod state;

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use fresnel_net::task::TaskHandle;
use tokio::net::TcpListener;

use crate::KitError;
use crate::background::Background;
use state::Shared;

/// How the proxy treats its connections. A change applies from the next byte and the next
/// connection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ProxyMode {
    /// All bytes pass.
    Pass,
    /// A new connection gets no answer for a random time between `min` and `max`, with this
    /// probability. Then it works. Open connections work.
    HoldNew {
        /// The part of the new connections that the proxy holds, from 0.0 to 1.0.
        probability: f64,
        /// The shortest hold time.
        min: Duration,
        /// The longest hold time.
        max: Duration,
    },
    /// Open connections stay open but carry no bytes and no close. A new connection gets no
    /// answer. The bytes pass when the mode changes.
    Freeze,
    /// Each byte arrives this time after the proxy read it, in each direction. The data rate
    /// has no limit. A new connection waits one round trip, as a TCP handshake does.
    Latency(Duration),
    /// The proxy closes all open connections with a TCP reset each time this period ends.
    ResetEach(Duration),
}

/// A copy of the counters of the proxy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProxyCounters {
    /// TCP connections that clients opened since the start.
    pub accepted: u64,
    /// New connections that the mode `HoldNew` held.
    pub held: u64,
    /// Connections that are open now. A silent connection stays open.
    pub open: u64,
    /// Connections that the proxy closed with a reset: by `reset_all` or by `ResetEach`.
    pub reset: u64,
    /// Connections that the proxy closed because the server did not accept its connect.
    pub refused: u64,
    /// Connections that ended with an error of a socket, for example a reset by a peer.
    pub broken: u64,
}

/// A running fault proxy. It stops when this value drops, and its connections close.
pub struct FaultProxy {
    addr: SocketAddr,
    shared: Arc<Shared>,
    background: Background,
    _driver: TaskHandle,
}

impl FaultProxy {
    /// Listens on a free port of 127.0.0.1 and forwards each connection to `upstream`.
    pub async fn start(upstream: SocketAddr) -> Result<Self, KitError> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let addr = listener.local_addr()?;
        let shared = Arc::new(Shared::new(listener, upstream));
        let background = Background::new()?;
        let driver = background.start("fault proxy", {
            let shared = Arc::clone(&shared);
            move || driver::run(Arc::clone(&shared))
        });
        Ok(Self { addr, shared, background, _driver: driver })
    }

    /// The address that the client connects to.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Changes the mode.
    pub fn set(&self, mode: ProxyMode) {
        self.shared.mode.send_replace(mode);
    }

    /// Makes each connection that is open now silent for all time: it carries no more bytes
    /// and no close. New connections work. This is the state after a NAT device lost its table.
    pub fn silence_open(&self) {
        self.shared.silence_open();
    }

    /// Closes all open connections with a TCP reset. A connection that opens after the return
    /// is not in danger. On macOS, the call first waits until each peer took its bytes, for
    /// 2 seconds at most for each socket.
    pub async fn reset_all(&self) {
        let request = self.shared.request_reset();
        let mut done = self.shared.resets_done.subscribe();
        // The sender lives in `shared`, so the wait cannot fail.
        if done.wait_for(|done| *done >= request).await.is_err() {
            std::future::pending::<()>().await;
        }
    }

    /// A copy of the counters.
    pub fn counters(&self) -> ProxyCounters {
        *self.shared.counters.borrow()
    }

    /// Waits until the counters meet the condition and gives that copy. A test waits for this
    /// signal and not for a fixed time.
    pub async fn wait_for(&self, mut condition: impl FnMut(&ProxyCounters) -> bool) -> ProxyCounters {
        let mut counters = self.shared.counters.subscribe();
        let met = counters.wait_for(|counters| condition(counters)).await.map(|counters| *counters);
        met.unwrap_or_else(|_| self.counters())
    }

    /// One line for each run of the proxy task that ended. An empty list is the normal case.
    pub fn task_failures(&self) -> Vec<String> {
        self.background.failures()
    }
}
