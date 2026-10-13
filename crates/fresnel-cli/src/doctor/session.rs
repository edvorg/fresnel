//! The connected cluster: its credential, its channels and the answers of its server.
//!
//! The session connects the cluster as the app does. It sends one request only, a number
//! of times: `GET /version`, which holds no data of an object.

use std::time::Duration;

use fresnel_core::cluster::{ClusterId, Link, Registry};
use fresnel_core::net::credential::CredentialState;
use fresnel_core::net::{ChannelState, ChannelStatus, ClusterStatus, GroupStatus};
use tokio::time::{Instant, timeout};

use super::report::{Channel, Credential, IdleClose, Session, Step, Summary, Version, ms, seconds};

/// The longest wait for the first answer on each channel, after the credential is ready.
const LIVE_LIMIT: Duration = Duration::from_secs(20);
/// The count of small requests for the round trip time.
const PINGS: usize = 5;
const VERSION_PATH: &str = "/version";

/// The limits of one session.
pub(super) struct Limits {
    /// The longest wait for the first token: the time limit of the plugin and a margin.
    pub(super) credential: Duration,
    /// The longest wait for the close of an idle connection. `None`: no such measurement.
    pub(super) idle_close: Option<Duration>,
}

/// Connects the cluster, measures and closes its connections again.
pub(super) async fn measure(registry: &Registry, id: &ClusterId, limits: &Limits) -> Step<Session> {
    let started = Instant::now();
    let link = match registry.connect(id) {
        Ok(link) => link,
        Err(error) => return Step::failed(error),
    };
    let session = examine(&link, started, limits).await;
    registry.disconnect(id);
    Step::Done(session)
}

async fn examine(link: &Link, started: Instant, limits: &Limits) -> Session {
    let settled = until(link, limits.credential, credential_settled).await;
    let credential = credential(&settled.unwrap_or_else(|| link.net.status()), started);
    let live = until(link, LIVE_LIMIT, all_live).await;
    let (live_ms, live_at) = (live.as_ref().map(|_| ms(started.elapsed())), Instant::now());
    let (version, round_trips) = match live {
        Some(_) => version_and_round_trips(link).await,
        None => (Step::failed(format!("no answer on each channel in {LIVE_LIMIT:?}")), Vec::new()),
    };
    // The state of the channels before the wait: the wait ends with a channel that connects again.
    let status = link.net.status();
    let idle_close = match (limits.idle_close, live_ms) {
        (Some(limit), Some(_)) => Some(idle_close(link, live_at, limit).await),
        (None, _) | (_, None) => None,
    };
    let groups = [("watches", &status.watches), ("requests", &status.requests), ("logs", &status.logs)];
    let channels: Vec<Channel> = groups
        .iter()
        .filter_map(|(purpose, group)| group.channels.first().map(|first| channel(purpose, first)))
        .collect();
    Session {
        credential,
        live_ms,
        stream_limit: channels.iter().filter_map(|channel| channel.stream_limit).min(),
        channels,
        ping_ms: Summary::of(&round_trips),
        version,
        idle_close,
    }
}

/// Waits until the status of the cluster meets the condition. `None` at the limit.
async fn until(link: &Link, limit: Duration, met: impl Fn(&ClusterStatus) -> bool) -> Option<ClusterStatus> {
    let mut changes = link.net.changes();
    let wait = async {
        loop {
            let status = link.net.status();
            if met(&status) {
                return status;
            }
            changes.next().await;
        }
    };
    timeout(limit, wait).await.ok()
}

/// True when the first plugin run ended, or when the cluster has no plugin.
fn credential_settled(status: &ClusterStatus) -> bool {
    let state = status.credential.as_ref().map(|credential| &credential.state);
    !matches!(state, Some(CredentialState::None | CredentialState::Running { .. }))
}

fn first_is(group: &GroupStatus, met: impl Fn(&ChannelStatus) -> bool) -> bool {
    group.channels.first().is_some_and(met)
}

fn all_live(status: &ClusterStatus) -> bool {
    [&status.watches, &status.requests, &status.logs]
        .iter()
        .all(|group| first_is(group, |channel| channel.state == ChannelState::Live))
}

fn credential(status: &ClusterStatus, started: Instant) -> Option<Credential> {
    let credential = status.credential.as_ref()?;
    let (valid, detail) = match &credential.state {
        CredentialState::Valid { .. } => (true, ""),
        CredentialState::Failed { failure, .. } => (false, failure.detail()),
        CredentialState::None | CredentialState::Running { .. } => (false, ""),
    };
    Some(Credential {
        plugin_runs: credential.plugin_runs,
        first_token_ms: valid.then(|| ms(started.elapsed())),
        state: credential.state.to_string(),
        detail: detail.to_owned(),
    })
}

/// Sends the small requests through the gateway. The first answer is the version; each
/// answer gives one round trip on the open request channel.
async fn version_and_round_trips(link: &Link) -> (Step<Version>, Vec<Duration>) {
    let mut round_trips = Vec::new();
    let mut version = None;
    for _ in 0..PINGS {
        let request = match http::Request::get(VERSION_PATH).body(Vec::new()) {
            Ok(request) => request,
            Err(error) => return (Step::failed(error), round_trips),
        };
        let sent = Instant::now();
        match link.gateway.get::<Version>(request).await {
            Ok(answer) => {
                round_trips.push(sent.elapsed());
                version.get_or_insert(answer);
            }
            Err(error) if version.is_none() => return (Step::failed(error), round_trips),
            Err(_) => break,
        }
    }
    let version = version.map_or_else(|| Step::failed("no request was sent"), Step::Done);
    (version, round_trips)
}

/// Waits for the close of the logs channel. The settings of this run give that channel no
/// light request, so it sends pings and nothing else after its first answer.
async fn idle_close(link: &Link, live_at: Instant, limit: Duration) -> IdleClose {
    let lost = |status: &ClusterStatus| {
        first_is(&status.logs, |channel| channel.connections >= 2 || channel.state != ChannelState::Live)
    };
    let closed = until(link, limit.saturating_sub(live_at.elapsed()), lost).await;
    let waited = live_at.elapsed();
    let status = link.net.status();
    let kept = |group: &GroupStatus| first_is(group, |channel| channel.connections == 1);
    IdleClose {
        waited_s: seconds(waited),
        closed_after_s: closed.as_ref().map(|_| seconds(waited)),
        reason: closed.and_then(|closed| closed.logs.channels.first().and_then(|channel| channel.last_error.clone())),
        light_request_kept_open: kept(&status.watches) && kept(&status.requests),
    }
}

fn channel(purpose: &'static str, status: &ChannelStatus) -> Channel {
    let state = match &status.state {
        ChannelState::Connecting { attempt } => format!("connecting, attempt {attempt}"),
        ChannelState::Verifying => "connected, no answer yet".to_owned(),
        ChannelState::Live => "live".to_owned(),
        ChannelState::Backoff { failures, .. } => format!("pause after {failures} failures"),
        ChannelState::Retired => "closed".to_owned(),
    };
    Channel {
        purpose,
        state,
        connections: status.connections,
        stream_limit: status.server_stream_limit,
        stream_budget: status.stream_budget,
        light_request_ms: status.ping_time.map(ms),
        last_error: status.last_error.clone(),
    }
}
