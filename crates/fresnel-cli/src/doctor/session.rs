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
    // With no token, a request fails with the state of the credential, and a channel sends
    // no light request, so no channel gets live. The wait for the channels would only
    // cost its limit.
    let has_token = credential.as_ref().is_none_or(|credential| credential.first_token_ms.is_some());
    let live = if has_token { until(link, LIVE_LIMIT, all_live).await } else { None };
    let live_ms = live.as_ref().map(|_| ms(started.elapsed()));
    let requests = match (has_token, live) {
        (true, None) => Requests::none(format!("no answer on each channel in {LIVE_LIMIT:?}")),
        (true, Some(_)) | (false, _) => small_requests(link).await,
    };
    // The report has the channels from before the idle wait: that wait ends with a channel
    // that connects again.
    let (status, idle_close) = match limits.idle_close.filter(|_| live_ms.is_some()) {
        Some(limit) => idle_close(link, limit).await,
        None => (link.net.status(), None),
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
        ping_ms: Summary::of(&requests.round_trips),
        ping_error: requests.later_error,
        version: requests.version,
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

/// The answers to the small requests of one session.
struct Requests {
    /// The first answer, or why no request got an answer.
    version: Step<Version>,
    /// The time of each answer on the open request channel.
    round_trips: Vec<Duration>,
    /// Why a request after the first answer got none.
    later_error: Option<String>,
}

impl Requests {
    fn none(reason: String) -> Self {
        Self { version: Step::failed(reason), round_trips: Vec::new(), later_error: None }
    }
}

/// Sends the small requests through the gateway, one after the other, to the first error.
async fn small_requests(link: &Link) -> Requests {
    let mut round_trips = Vec::new();
    let (mut version, mut error) = (None, None);
    for _ in 0..PINGS {
        let sent = Instant::now();
        match version_request(link).await {
            Ok(answer) => {
                round_trips.push(sent.elapsed());
                version.get_or_insert(answer);
            }
            Err(text) => {
                error = Some(text);
                break;
            }
        }
    }
    match version {
        Some(version) => Requests { version: Step::Done(version), round_trips, later_error: error },
        None => Requests::none(error.unwrap_or_else(|| "no request was sent".to_owned())),
    }
}

async fn version_request(link: &Link) -> Result<Version, String> {
    let request = http::Request::get(VERSION_PATH).body(Vec::new()).map_err(|error| error.to_string())?;
    link.gateway.get::<Version>(request).await.map_err(|error| error.to_string())
}

/// Waits for the close of the logs channel. The settings of this run give that channel no
/// light request, so it sends pings and nothing else after its first answer. `None` when
/// the channels are not live at the start of the wait.
///
/// The wait starts from the connections of its own start. A channel can be on a later
/// connection then, for example after a reset during the start, and that loss is not the
/// close of an idle connection.
async fn idle_close(link: &Link, limit: Duration) -> (ClusterStatus, Option<IdleClose>) {
    let Some(before) = until(link, LIVE_LIMIT, all_live).await else {
        return (link.net.status(), None);
    };
    let started = Instant::now();
    let connection = |group: &GroupStatus| group.channels.first().map(|channel| channel.connections);
    let lost = |status: &ClusterStatus| {
        connection(&status.logs) != connection(&before.logs)
            || !first_is(&status.logs, |channel| channel.state == ChannelState::Live)
    };
    let closed = until(link, limit, lost).await;
    let waited = started.elapsed();
    // The connection was idle from the last frame of its first answer.
    let idle_before = before.logs.channels.first().and_then(|channel| channel.last_frame_age).unwrap_or_default();
    let status = link.net.status();
    let kept = |now: &GroupStatus, then: &GroupStatus| connection(now) == connection(then);
    let idle = IdleClose {
        waited_s: seconds(waited),
        closed_after_s: closed.as_ref().map(|_| seconds(idle_before + waited)),
        reason: closed.and_then(|closed| closed.logs.channels.first().and_then(|channel| channel.last_error.clone())),
        light_request_kept_open: kept(&status.watches, &before.watches) && kept(&status.requests, &before.requests),
    };
    (before, Some(idle))
}

fn channel(purpose: &'static str, status: &ChannelStatus) -> Channel {
    let state = match &status.state {
        ChannelState::Connecting { attempt } => format!("connecting, attempt {attempt}"),
        ChannelState::WaitsForToken => "connected, waits for a token".to_owned(),
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
