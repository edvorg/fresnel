//! New connections to one cluster (plan sections 8.3 and 8.4).
//!
//! - [`resolver`]: the address lookup with a time limit and the last good addresses.
//! - [`hedge`]: the race of connect attempts. [`times`]: the measured connect time.
//! - [`dial`]: one attempt, TCP and TLS.

pub(crate) mod dial;
mod hedge;
mod resolver;
mod times;

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use futures_util::FutureExt as _;
use futures_util::future::BoxFuture;
use rustls::ClientConfig;
use rustls::pki_types::ServerName;
use tokio::time::Instant;

use crate::error::NetError;
use dial::{Io, Protocol, Tls};
use hedge::Race;
use resolver::Resolver;
use times::ConnectTimes;

/// Where the API server of a cluster is, and how the app trusts it.
#[derive(Clone, Debug)]
pub struct Endpoint {
    /// The host of the server URL: a name or an address.
    pub host: String,
    /// The port of the server URL.
    pub port: u16,
    /// The path of the server URL, for a cluster behind a proxy with a path. Most often empty.
    pub base_path: String,
    /// The name in the certificate of the server, if it is not the host.
    pub tls_server_name: Option<String>,
    /// The trusted certificates and, for a cluster with client certificates, the identity.
    pub tls: Arc<ClientConfig>,
}

/// The settings of a connect. Each number is a first value (plan section 16).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectSettings {
    /// The time limit of the address lookup.
    pub resolve_limit: Duration,
    /// The hedge delay in percent of the measured connect time.
    pub hedge_percent: u32,
    /// The shortest hedge delay.
    pub hedge_min: Duration,
    /// The longest hedge delay.
    pub hedge_max: Duration,
    /// The hedge delay of a cluster with no measured connect time.
    pub hedge_first: Duration,
    /// The most connect attempts that run at the same time.
    pub max_parallel: usize,
    /// An attempt with no answer ends after this time.
    pub attempt_limit: Duration,
    /// The time limit of one full connect: lookup, TCP and TLS.
    pub connect_limit: Duration,
    /// A connect ends after this count of fast failures, for example refused connects.
    pub hard_failures: usize,
}

impl Default for ConnectSettings {
    fn default() -> Self {
        Self {
            resolve_limit: Duration::from_secs(2),
            hedge_percent: 150,
            hedge_min: Duration::from_millis(150),
            hedge_max: Duration::from_secs(1),
            // With 5 attempts 200 ms apart, a connect passes 1 s only when 5 attempts in
            // sequence get no answer.
            hedge_first: Duration::from_millis(200),
            max_parallel: 5,
            attempt_limit: Duration::from_secs(3),
            connect_limit: Duration::from_secs(10),
            hard_failures: 2,
        }
    }
}

/// A copy of the connect facts of one cluster.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectStatus {
    /// The last good addresses of the server.
    pub addresses: Vec<SocketAddr>,
    /// The error of the last address lookup, if it failed.
    pub resolve_error: Option<String>,
    /// The typical time of a connect: TCP plus TLS.
    pub connect_time: Option<Duration>,
    /// The delay before one more connect attempt starts.
    pub hedge_delay: Duration,
    /// The count of connects that gave a connection.
    pub connects: u64,
    /// The count of attempts for those connects. More attempts than connects shows held connections.
    pub attempts: u64,
}

/// Opens new connections. Channels and the upgrade client of a cluster share one value, so
/// that each connect uses the hedge and the same measured time.
pub(crate) trait Dial: Send + Sync + 'static {
    fn dial(&self, protocol: Protocol) -> BoxFuture<'_, Result<Io, NetError>>;

    /// A copy of the connect facts.
    fn status(&self) -> ConnectStatus;
}

/// The connector of one cluster.
pub(crate) struct Connector {
    settings: ConnectSettings,
    resolver: Resolver,
    tls: Tls,
    times: ConnectTimes,
    connects: AtomicU64,
    attempts: AtomicU64,
}

impl Connector {
    pub(crate) fn new(endpoint: &Endpoint, settings: ConnectSettings) -> Result<Self, NetError> {
        let name = endpoint.tls_server_name.as_ref().unwrap_or(&endpoint.host);
        let name = ServerName::try_from(name.clone())
            .map_err(|_| NetError::invalid(format!("`{name}` is not a name for a server certificate")))?;
        Ok(Self {
            resolver: Resolver::new(endpoint.host.clone(), endpoint.port, settings.resolve_limit),
            tls: Tls::new(&endpoint.tls, name),
            times: ConnectTimes::new(),
            connects: AtomicU64::new(0),
            attempts: AtomicU64::new(0),
            settings,
        })
    }

    /// One connect: the lookup, then the race. One time limit covers the two steps.
    async fn connect(&self, protocol: Protocol) -> Result<Io, NetError> {
        let started = Instant::now();
        let addresses = self.resolver.resolve().await?;
        let plan = Race {
            delay: self.times.hedge_delay(&self.settings),
            max_parallel: self.settings.max_parallel,
            attempt_limit: self.settings.attempt_limit,
            limit: self.settings.connect_limit.saturating_sub(started.elapsed()),
            hard_failures: self.settings.hard_failures,
        };
        // Each attempt takes the next address, so a host with one dead address still connects.
        let targets = addresses.iter().copied().cycle();
        let mut targets = targets.map(|address| self.tls.attempt(address, protocol));
        let attempt = |_| {
            let next = targets.next();
            async move {
                match next {
                    Some(connect) => connect.await,
                    None => Err(std::io::Error::other("the host has no address")),
                }
            }
        };
        let won = hedge::race(plan, attempt).await?;
        self.times.record(won.time);
        self.connects.fetch_add(1, Ordering::Relaxed);
        self.attempts.fetch_add(u64::try_from(won.attempts).unwrap_or(u64::MAX), Ordering::Relaxed);
        Ok(won.io)
    }
}

impl Dial for Connector {
    fn dial(&self, protocol: Protocol) -> BoxFuture<'_, Result<Io, NetError>> {
        self.connect(protocol).boxed()
    }

    fn status(&self) -> ConnectStatus {
        let (addresses, resolve_error) = self.resolver.status();
        ConnectStatus {
            addresses,
            resolve_error,
            connect_time: self.times.typical(),
            hedge_delay: self.times.hedge_delay(&self.settings),
            connects: self.connects.load(Ordering::Relaxed),
            attempts: self.attempts.load(Ordering::Relaxed),
        }
    }
}
