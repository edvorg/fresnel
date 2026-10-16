//! One cluster: its channels, its upgrade client, its credential and its tower service.
//!
//! This is the type that the crate above uses. It builds each part of the connection layer
//! for one cluster and gives copies of their states.

mod address;
mod light;
mod service;

use std::sync::{Arc, Weak};
use std::task::{Context, Poll};
use std::time::SystemTime;

use bytes::Bytes;
use futures_util::FutureExt as _;
use futures_util::future::BoxFuture;
use http::{Request, Response};
use hyper_util::rt::TokioIo;
use tokio::time::timeout;

use crate::body::ResponseBody;
use crate::changes::{Changed, Changes};
use crate::channel::group::{Group, GroupStatus};
use crate::channel::outage::{Outage, OutageSettings, OutageStatus};
use crate::channel::{ChannelSettings, ChannelState, ChannelStatus, Parts, Purpose};
use crate::clock::WallClock;
use crate::connect::dial::Protocol;
use crate::connect::{ConnectSettings, ConnectStatus, Connector, Dial, Endpoint};
use crate::credential::{
    CredentialSettings, CredentialStatus, Credentials, ExecPlugin, PluginSpec, Proof, Prover, TokenSource,
};
use crate::env::ProcessEnv;
use crate::error::NetError;
use crate::light::LightRequests;
use crate::request::Limits;
use crate::signal::Signal;
use crate::socket::{Offer, Path, Socket, SocketParts, SocketSpec};
use crate::task::{Backoff, Restart, TaskHandle, TaskName, Tasks};
use crate::throttle::{Throttle, ThrottleSettings, ThrottleStatus};
use crate::upgrade::{UpgradeClient, UpgradeSettings, UpgradeStatus, WarmGuard};
use address::Address;
use light::ClusterLight;

/// Each setting of the connection layer for one cluster.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetSettings {
    /// New connections.
    pub connect: ConnectSettings,
    /// The pace of the connects in an outage.
    pub outage: OutageSettings,
    /// The channels for watches.
    pub watches: ChannelSettings,
    /// The channels for lists, reads and writes.
    pub requests: ChannelSettings,
    /// The channels for log streams.
    pub logs: ChannelSettings,
    /// The credential plugin.
    pub credential: CredentialSettings,
    /// The time limits of the requests.
    pub limits: Limits,
    /// The upgrade connections.
    pub upgrade: UpgradeSettings,
    /// The throttle of the server.
    pub throttle: ThrottleSettings,
    /// The path of the light request. Each user can read it, and a bad token gets HTTP 401.
    pub light_path: String,
    /// The value of the `User-Agent` header.
    pub user_agent: String,
}

impl Default for NetSettings {
    fn default() -> Self {
        Self {
            connect: ConnectSettings::default(),
            outage: OutageSettings::default(),
            watches: ChannelSettings::for_watches(),
            requests: ChannelSettings::for_requests(),
            logs: ChannelSettings::for_logs(),
            credential: CredentialSettings::default(),
            limits: Limits::default(),
            upgrade: UpgradeSettings::default(),
            throttle: ThrottleSettings::default(),
            light_path: "/version".to_owned(),
            user_agent: concat!("fresnel/", env!("CARGO_PKG_VERSION")).to_owned(),
        }
    }
}

/// What the connection layer needs to know about one cluster.
#[derive(Clone, Debug)]
pub struct ClusterSpec {
    /// The name of the cluster: its kubeconfig context. Each task of the cluster carries it.
    pub name: String,
    /// Where the API server is.
    pub endpoint: Endpoint,
    /// The exec plugin of the kubeconfig user. `None` for a cluster whose requests carry no
    /// token: a client certificate in the TLS settings proves the user.
    pub plugin: Option<PluginSpec>,
    /// The environment for the process of the plugin: the result of [`crate::env::resolve`].
    pub env: ProcessEnv,
    /// The settings.
    pub settings: NetSettings,
    /// The wall clock. A test gives its own.
    pub clock: WallClock,
}

/// A copy of each state of one cluster, for the Connections view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClusterStatus {
    /// The name of the cluster.
    pub name: String,
    /// New connections.
    pub connect: ConnectStatus,
    /// The outage: no connect of a channel worked since its start. `None` with no outage.
    pub outage: Option<OutageStatus>,
    /// The channels for watches.
    pub watches: GroupStatus,
    /// The channels for lists, reads and writes.
    pub requests: GroupStatus,
    /// The channels for log streams.
    pub logs: GroupStatus,
    /// The credential. `None` for a cluster with no plugin.
    pub credential: Option<CredentialStatus>,
    /// The upgrade connections.
    pub upgrade: UpgradeStatus,
    /// The throttle of the server.
    pub throttle: ThrottleStatus,
}

struct Inner {
    name: Arc<str>,
    address: Arc<Address>,
    limits: Limits,
    dial: Arc<dyn Dial>,
    outage: Arc<Outage>,
    watches: Arc<Group>,
    requests: Arc<Group>,
    logs: Arc<Group>,
    upgrade: UpgradeClient,
    sockets: SocketParts,
    credentials: Option<Credentials>,
    throttle: Throttle,
    signal: Signal,
    changed: Changed,
    _credential_task: Option<TaskHandle>,
}

/// The connections of one cluster. A clone is one more handle; the connections close and
/// the tasks stop when the last handle drops.
#[derive(Clone)]
pub struct Cluster {
    inner: Arc<Inner>,
}

impl Cluster {
    /// Starts the tasks of the cluster. Each channel connects at once, and the plugin runs
    /// at once, so that the first request finds them ready. The call needs no runtime context.
    pub fn start(tasks: &Tasks, spec: ClusterSpec) -> Result<Self, NetError> {
        let ClusterSpec { name, endpoint, plugin, env, settings, clock } = spec;
        let address = Address::new(&endpoint, &settings.user_agent, &settings.light_path)?;
        let dial = Arc::new(Connector::new(&endpoint, settings.connect.clone())?);
        let source = plugin.map(|plugin| Arc::new(ExecPlugin::new(plugin, env, clock.clone())) as Arc<dyn TokenSource>);
        Ok(Self::build(tasks, name.into(), address, dial, source, settings, clock))
    }

    /// Builds the parts of the cluster on a dialer and on a token source.
    fn build(
        tasks: &Tasks,
        name: Arc<str>,
        address: Address,
        dial: Arc<dyn Dial>,
        source: Option<Arc<dyn TokenSource>>,
        settings: NetSettings,
        clock: WallClock,
    ) -> Self {
        let address = Arc::new(address);
        let (signal, changed) = (Signal::new(), Changed::new());
        let credential = |_| Credentials::new(settings.credential.clone(), clock.clone(), changed.clone());
        let credentials = source.as_ref().map(credential);
        let light: Arc<dyn LightRequests> =
            Arc::new(ClusterLight { address: Arc::clone(&address), credentials: credentials.clone() });
        let outage = Arc::new(Outage::new(settings.outage, &signal, changed.clone()));
        let parts = Parts {
            tasks: tasks.clone(),
            cluster: Arc::clone(&name),
            dial: Arc::clone(&dial),
            light,
            signal: signal.clone(),
            changed: changed.clone(),
            outage: Arc::clone(&outage),
        };
        // The channels of the three purposes connect at the same time.
        let group = |purpose, settings| Arc::new(Group::start(parts.clone(), purpose, settings));
        let (watches, logs) = (group(Purpose::Watches, settings.watches), group(Purpose::Logs, settings.logs));
        let requests = group(Purpose::Requests, settings.requests);

        let upgrade = UpgradeClient::start(parts, settings.upgrade.clone());
        let sockets = SocketParts {
            tasks: tasks.clone(),
            cluster: Arc::clone(&name),
            settings: settings.upgrade.socket,
            signal: signal.clone(),
            path: Arc::new(ChannelPath([&watches, &requests, &logs].map(Arc::downgrade))),
            changed: changed.clone(),
            counts: upgrade.socket_counts(),
        };
        let credential_task = credentials.clone().zip(source).map(|(credentials, source)| {
            let limit = settings.credential.proof_limit;
            let prover =
                Arc::new(ProofRequest { requests: Arc::clone(&requests), address: Arc::clone(&address), limit });
            let signal = signal.clone();
            let body = move || credentials.task(Arc::clone(&source), Arc::clone(&prover) as _, &signal);
            tasks.start(TaskName::scoped("credential", Arc::clone(&name)), Restart::Always(Backoff::default()), body)
        });
        let inner = Inner {
            name,
            address,
            limits: settings.limits,
            dial,
            outage,
            watches,
            requests,
            logs,
            upgrade,
            sockets,
            credentials,
            throttle: Throttle::new(settings.throttle, clock, changed.clone()),
            signal,
            changed,
            _credential_task: credential_task,
        };
        Self { inner: Arc::new(inner) }
    }

    /// Sends one request and waits for the head of its answer. The request carries its
    /// [`RequestKind`](crate::RequestKind) as an extension, and its path with no host.
    pub async fn send(&self, request: Request<Bytes>) -> Result<Response<ResponseBody>, NetError> {
        self.inner.send(request).await
    }

    /// A copy of each state of the cluster.
    pub fn status(&self) -> ClusterStatus {
        let inner = &self.inner;
        ClusterStatus {
            name: inner.name.to_string(),
            connect: inner.dial.status(),
            outage: inner.outage.status(),
            watches: inner.watches.status(),
            requests: inner.requests.status(),
            logs: inner.logs.status(),
            credential: inner.credentials.as_ref().map(Credentials::status),
            upgrade: inner.upgrade.status(),
            throttle: inner.throttle.status(),
        }
    }

    /// Tells when a state of the cluster changed, so that the UI reads [`Cluster::status`] again.
    pub fn changes(&self) -> Changes {
        self.inner.changed.watch()
    }

    /// The signal of the cluster. A raise ends each retry delay: a channel tests its
    /// connection, and a failed credential runs its plugin now. A channel with no connection
    /// connects now, one time in [`OutageSettings::calm`] at most. "Retry now", a wake and a
    /// network change raise it.
    pub fn signal(&self) -> Signal {
        self.inner.signal.clone()
    }

    /// Keeps idle upgrade connections ready while the guard lives. A forward and a terminal
    /// each hold one guard.
    pub fn keep_upgrades_warm(&self) -> WarmGuard {
        self.inner.upgrade.keep_warm()
    }

    /// Changes the answer HTTP 101 to an upgrade request with the headers of `offer` into a
    /// WebSocket. Its task has the ping of the cluster settings, and the counters of the
    /// cluster count it. The caller reads the message of a refusal before this call.
    pub async fn open_socket<B>(
        &self,
        response: Response<B>,
        offer: &Offer,
        spec: SocketSpec,
    ) -> Result<Socket, NetError> {
        let protocol = offer.accept(&response)?;
        let upgraded = hyper::upgrade::on(response).await;
        let upgraded = upgraded.map_err(|error| NetError::because("the upgraded connection", &error))?;
        Ok(Socket::start(TokioIo::new(upgraded), protocol, &self.inner.sockets, spec).await)
    }
}

/// The path of the sockets of a cluster: its channels. A channel that has been live since the
/// last frame of a socket shows that the path of that time answers, so the write of a socket
/// that waits waits for its peer and not for the network. A socket does not keep the channels
/// of a cluster that is gone: then no path is live.
struct ChannelPath([Weak<Group>; 3]);

impl Path for ChannelPath {
    fn live_since(&self, since: SystemTime) -> bool {
        // `since` of a live channel is the time when its connection got its first answer.
        let from_before = |channel: &ChannelStatus| channel.state == ChannelState::Live && channel.since <= since;
        let any = |group: Arc<Group>| group.status().channels.iter().any(from_before);
        self.0.iter().filter_map(Weak::upgrade).any(any)
    }
}

impl std::fmt::Debug for Cluster {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cluster").field("name", &self.inner.name).finish_non_exhaustive()
    }
}

/// The tower service of the cluster. The service is always ready: a request waits inside
/// its call, where the wait has a time limit and a state.
impl tower_service::Service<Request<Bytes>> for Cluster {
    type Response = Response<ResponseBody>;
    type Error = NetError;
    type Future = BoxFuture<'static, Result<Response<ResponseBody>, NetError>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), NetError>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: Request<Bytes>) -> Self::Future {
        let inner = Arc::clone(&self.inner);
        async move { inner.send(request).await }.boxed()
    }
}

/// The proof of a token: the light request with that token, on a request channel.
struct ProofRequest {
    requests: Arc<Group>,
    address: Arc<Address>,
    limit: std::time::Duration,
}

impl Prover for ProofRequest {
    fn prove(&self, authorization: http::HeaderValue) -> BoxFuture<'_, Proof> {
        async move {
            let request = self.address.light_request(Protocol::Http2, Some(authorization));
            let exchange = async {
                let (channel, _stream) = self.requests.stream().await;
                channel.send(request).await
            };
            match timeout(self.limit, exchange).await {
                Ok(Ok(answer)) if answer.status() == http::StatusCode::UNAUTHORIZED => Proof::Refused,
                Ok(Ok(_)) => Proof::Accepted,
                Ok(Err(_)) | Err(_) => Proof::NoAnswer,
            }
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests;
