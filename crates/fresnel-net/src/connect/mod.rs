//! New connections to one cluster (plan sections 8.3 and 8.4).
//!
//! - [`resolver`]: the address lookup with a time limit and the last good addresses.
//! - [`hedge`]: the race of connect attempts. [`times`]: the measured connect time.
//! - [`dial`]: one attempt, TCP and TLS.

pub(crate) mod dial;
mod hedge;
pub(crate) mod resolver;
mod times;

use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use futures_util::FutureExt as _;
use futures_util::future::BoxFuture;
use rustls::ClientConfig;
use rustls::pki_types::ServerName;
use tokio::time::Instant;

use crate::error::NetError;
use dial::{Io, Protocol, Tls};
use hedge::{Race, Won};
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
    /// A connect ends after this count of fast failures, for example refused connects. A
    /// host with more addresses than this gets one attempt for each address.
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

/// How hard one connect tries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Effort {
    /// The hedged race (plan section 8.4): a fast connect also on a path that holds some.
    Race,
    /// One attempt, to the next address of the host in turn. In a long outage the channels
    /// test the path with it, so that each turn of the cluster costs one attempt.
    Single,
}

/// Opens new connections. Channels and the upgrade client of a cluster share one value, so
/// that each connect uses the hedge and the same measured time.
pub(crate) trait Dial: Send + Sync + 'static {
    fn dial(&self, protocol: Protocol, effort: Effort) -> BoxFuture<'_, Result<Io, NetError>>;

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
    /// The count of single attempts. Each one goes to the next address, so that a host with
    /// one dead address still connects in a long outage.
    singles: AtomicUsize,
}

impl Connector {
    pub(crate) fn new(endpoint: &Endpoint, settings: ConnectSettings) -> Result<Self, NetError> {
        Ok(Self {
            resolver: Resolver::new(endpoint.host.clone(), endpoint.port, settings.resolve_limit),
            tls: Tls::new(&endpoint.tls, server_name(endpoint)?),
            times: ConnectTimes::new(),
            connects: AtomicU64::new(0),
            attempts: AtomicU64::new(0),
            singles: AtomicUsize::new(0),
            settings,
        })
    }

    /// One connect: the lookup, then the race. One time limit covers the two steps. Gives
    /// the connection and the count of attempts that started.
    pub(crate) async fn connect(&self, protocol: Protocol, effort: Effort) -> Result<(Io, usize), NetError> {
        let started = Instant::now();
        let addresses = self.resolver.resolve().await?;
        let (max_attempts, first) = match effort {
            Effort::Race => (usize::MAX, 0),
            Effort::Single => (1, self.singles.fetch_add(1, Ordering::Relaxed)),
        };
        let plan = Race {
            delay: self.times.hedge_delay(&self.settings),
            max_parallel: self.settings.max_parallel,
            attempt_limit: self.settings.attempt_limit,
            limit: self.settings.connect_limit.saturating_sub(started.elapsed()),
            hard_failures: self.settings.hard_failures,
            max_attempts,
        };
        let won = race_over(plan, &addresses, first, |address| self.tls.attempt(address, protocol)).await?;
        self.times.record(won.time);
        self.connects.fetch_add(1, Ordering::Relaxed);
        self.attempts.fetch_add(u64::try_from(won.attempts).unwrap_or(u64::MAX), Ordering::Relaxed);
        Ok((won.io, won.attempts))
    }
}

/// The name that the certificate of the server must hold.
pub(crate) fn server_name(endpoint: &Endpoint) -> Result<ServerName<'static>, NetError> {
    let name = endpoint.tls_server_name.as_ref().unwrap_or(&endpoint.host);
    ServerName::try_from(name.clone())
        .map_err(|_| NetError::invalid(format!("`{name}` is not a name for a server certificate")))
}

/// The race over the addresses of one host. Attempt N goes to address `first` + N, in a
/// circle, so that a host with dead addresses still connects. Each address gets its attempt
/// before fast failures end the race.
async fn race_over<T, F>(
    plan: Race,
    addresses: &[SocketAddr],
    first: usize,
    connect: impl Fn(SocketAddr) -> F,
) -> Result<Won<T>, NetError>
where
    F: Future<Output = io::Result<T>>,
{
    let plan = Race { hard_failures: plan.hard_failures.max(addresses.len()), ..plan };
    let attempt = |number: usize| {
        let target = addresses.get(first.wrapping_add(number) % addresses.len().max(1)).copied().map(&connect);
        async move {
            match target {
                Some(connect) => connect.await,
                None => Err(io::Error::other("the host has no address")),
            }
        }
    };
    hedge::race(plan, attempt).await
}

impl Dial for Connector {
    fn dial(&self, protocol: Protocol, effort: Effort) -> BoxFuture<'_, Result<Io, NetError>> {
        self.connect(protocol, effort).map(|connected| connected.map(|(io, _attempts)| io)).boxed()
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

#[cfg(test)]
mod tests {
    use tokio::time::sleep;

    use super::*;
    use crate::error::ErrorClass;

    const MS: Duration = Duration::from_millis(1);
    const PLAN: Race = Race {
        delay: Duration::from_millis(200),
        max_parallel: 5,
        attempt_limit: Duration::from_secs(3),
        limit: Duration::from_secs(10),
        hard_failures: 2,
        max_attempts: usize::MAX,
    };

    fn address(last: u8) -> SocketAddr {
        SocketAddr::from(([10, 0, 0, last], 6443))
    }

    /// A race where only the address `good` connects. Each other address refuses after 1 ms.
    async fn connect_to(
        addresses: &[SocketAddr],
        good: Option<SocketAddr>,
    ) -> (Result<Won<SocketAddr>, NetError>, Duration) {
        race_from(PLAN, addresses, 0, good).await
    }

    async fn race_from(
        plan: Race,
        addresses: &[SocketAddr],
        first: usize,
        good: Option<SocketAddr>,
    ) -> (Result<Won<SocketAddr>, NetError>, Duration) {
        let started = Instant::now();
        let connect = |address| async move {
            sleep(MS).await;
            if Some(address) == good { Ok(address) } else { Err(io::Error::other("connection refused")) }
        };
        (race_over(plan, addresses, first, connect).await, started.elapsed())
    }

    #[tokio::test(start_paused = true)]
    async fn each_address_of_a_host_gets_its_attempt() {
        let addresses = [address(1), address(2), address(3)];
        let (won, elapsed) = connect_to(&addresses, Some(address(3))).await;
        let won = won.expect("two dead addresses come before the good one");
        assert_eq!((won.io, won.attempts, elapsed), (address(3), 3, 401 * MS));

        let (lost, elapsed) = connect_to(&addresses, None).await;
        let error = lost.expect_err("no address accepts a connection");
        assert_eq!((error.class(), elapsed), (ErrorClass::Network, 401 * MS), "no wait for the time limit: {error}");

        let (lost, elapsed) = connect_to(&addresses[..1], None).await;
        assert!(lost.expect_err("refused").to_string().contains("2 attempts"));
        assert_eq!(elapsed, 201 * MS, "one address gets the count of fast failures of the settings");
    }

    #[tokio::test(start_paused = true)]
    async fn each_single_attempt_goes_to_the_next_address() {
        let (addresses, single) = ([address(1), address(2), address(3)], Race { max_attempts: 1, ..PLAN });
        let mut tried = Vec::new();
        for first in [0, 1, 2, 3, usize::MAX] {
            let (result, elapsed) = race_from(single, &addresses, first, Some(address(2))).await;
            assert_eq!(elapsed, MS, "one attempt, with no second one after the delay");
            tried.push(result.is_ok());
        }
        assert_eq!(tried, [false, true, false, false, false], "only the attempt to the second address connects");
    }
}
