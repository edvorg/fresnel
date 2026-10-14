//! Measured single steps of a connect, for fault diagnosis.
//!
//! The `doctor` command measures an endpoint with the connect code of the app, so that its
//! numbers hold for the app. A probe sends no HTTP request: each connection closes after
//! its TLS handshake.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use tokio::time::{Instant, timeout};

use crate::connect::dial::{self, Protocol, Tls};
use crate::connect::resolver::system_lookup;
use crate::connect::{ConnectSettings, ConnectStatus, Connector, Dial as _, Effort, Endpoint, server_name};
use crate::error::NetError;

/// The result of one address lookup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lookup {
    /// The time of the lookup. Zero for a host that is an address: it needs no lookup.
    pub time: Duration,
    /// Each address of the host.
    pub addresses: Vec<SocketAddr>,
}

/// The times of one connect attempt with no hedge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlainConnect {
    /// The time of the TCP connect.
    pub tcp: Duration,
    /// The time of the TLS handshake after it.
    pub tls: Duration,
}

/// One connect as a channel of the app makes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HedgedConnect {
    /// The time from the start to a connection after TLS, with the lookup.
    pub time: Duration,
    /// The count of attempts that started. More than one shows a slow or a held attempt.
    pub attempts: usize,
}

/// Measures the steps of a connect to one cluster.
pub struct Probe {
    host: String,
    port: u16,
    settings: ConnectSettings,
    tls: Tls,
    connector: Connector,
}

impl Probe {
    /// A probe for one endpoint. The settings are those of the connects of the app.
    pub fn new(endpoint: &Endpoint, settings: ConnectSettings) -> Result<Self, NetError> {
        Ok(Self {
            host: endpoint.host.clone(),
            port: endpoint.port,
            tls: Tls::new(&endpoint.tls, server_name(endpoint)?),
            connector: Connector::new(endpoint, settings.clone())?,
            settings,
        })
    }

    /// The address lookup of the system, with the time limit of the settings.
    pub async fn lookup(&self) -> Result<Lookup, NetError> {
        if let Ok(ip) = self.host.parse::<IpAddr>() {
            return Ok(Lookup { time: Duration::ZERO, addresses: vec![SocketAddr::new(ip, self.port)] });
        }
        let (started, limit) = (Instant::now(), self.settings.resolve_limit);
        let problem = match timeout(limit, system_lookup(self.host.clone(), self.port)).await {
            Ok(Ok(addresses)) if !addresses.is_empty() => return Ok(Lookup { time: started.elapsed(), addresses }),
            Ok(Ok(_)) => "the name has no address".to_owned(),
            Ok(Err(error)) => error,
            Err(_) => return Err(NetError::timeout(format!("cannot resolve {}: no answer in {limit:?}", self.host))),
        };
        Err(NetError::network(format!("cannot resolve {}: {problem}", self.host)))
    }

    /// One connect attempt to one address with no hedge: TCP, then TLS with one protocol.
    /// An attempt with no answer in `limit` is a held connection, and it ends there.
    pub async fn plain(
        &self,
        address: SocketAddr,
        protocol: Protocol,
        limit: Duration,
    ) -> Result<PlainConnect, NetError> {
        let attempt = async {
            let started = Instant::now();
            let tcp = dial::open(address).await.map_err(|error| NetError::because("the TCP connect failed", &error))?;
            let tcp_time = started.elapsed();
            let handshake = self.tls.handshake(tcp, protocol).await;
            handshake.map_err(|error| NetError::because("the TLS handshake failed", &error))?;
            Ok(PlainConnect { tcp: tcp_time, tls: started.elapsed().saturating_sub(tcp_time) })
        };
        timeout(limit, attempt).await.unwrap_or_else(|_| Err(NetError::timeout(format!("no answer in {limit:?}"))))
    }

    /// One connect as a channel makes it: the lookup with the last good addresses, then the
    /// hedged race. The hedge delay follows the connects of this probe.
    pub async fn hedged(&self, protocol: Protocol) -> Result<HedgedConnect, NetError> {
        let started = Instant::now();
        let (_connection, attempts) = self.connector.connect(protocol, Effort::Race).await?;
        Ok(HedgedConnect { time: started.elapsed(), attempts })
    }

    /// The connect facts of this probe: the typical connect time and the hedge delay.
    pub fn status(&self) -> ConnectStatus {
        self.connector.status()
    }
}

impl std::fmt::Debug for Probe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Probe").field("host", &self.host).field("port", &self.port).finish_non_exhaustive()
    }
}
