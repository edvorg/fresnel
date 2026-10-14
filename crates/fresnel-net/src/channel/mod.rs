//! Channels: the long-lived HTTP/2 connections of a cluster (plan sections 8.5 to 8.7).
//!
//! - [`Channel`]: one connection with its own connect task ([`driver`]) and one state.
//! - [`group`]: the channels of one purpose, with the stream budget.
//! - [`connection`]: the hyper connection. [`link`]: the one shared result of a connect.
//! - [`streams`]: the count of open streams. [`outage`]: the pace of connects in an outage.

mod connection;
mod driver;
pub(crate) mod group;
mod link;
pub(crate) mod outage;
mod state;
pub(crate) mod streams;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use bytes::Bytes;
use http::{Request, Response};
use http_body_util::Full;
use hyper::body::Incoming;
use tokio::sync::Notify;

pub use state::{ChannelState, ChannelStatus};

use crate::changes::Changed;
use crate::connect::Dial;
use crate::error::NetError;
use crate::light::LightRequests;
use crate::signal::Signal;
use crate::task::{Backoff, Restart, TaskHandle, TaskName, Tasks};
use link::LinkBoard;
use outage::Outage;
use state::StatusBoard;
use streams::Streams;

/// The settings of the channels of one purpose. Each number is a first value (plan section 16).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelSettings {
    /// A ping goes out when no frame arrived for this time.
    pub keepalive_interval: Duration,
    /// The connection closes when a ping gets no answer in this time.
    pub keepalive_timeout: Duration,
    /// A channel with no open stream sends one light request each time this period ends.
    /// A kube-apiserver closes a connection with no request after 90 s.
    pub light_period: Duration,
    /// The time limit for the answer to the first request and to a light request.
    pub answer_limit: Duration,
    /// The time limit for the answer to the test after a wake or a network change.
    pub test_limit: Duration,
    /// The fixed receive window of one stream.
    pub stream_window: u32,
    /// The fixed receive window of the connection.
    pub connection_window: u32,
    /// The part of the stream limit of the server that a channel uses, in percent.
    pub stream_budget_percent: usize,
    /// The most connections of this purpose. One more opens when each one is at its budget.
    pub max_connections: usize,
    /// An extra connection with no stream for this time closes.
    pub extra_idle: Duration,
    /// How often the channel reads the stream limit of the server and looks for GOAWAY.
    pub poll_period: Duration,
    /// The pauses between connects after a failure. A connection that was live for
    /// `reset_after` or longer connects again with no pause.
    pub backoff: Backoff,
}

impl ChannelSettings {
    /// For watches: small events on many streams.
    pub fn for_watches() -> Self {
        Self {
            keepalive_interval: Duration::from_secs(10),
            keepalive_timeout: Duration::from_secs(5),
            light_period: Duration::from_secs(30),
            answer_limit: Duration::from_secs(10),
            test_limit: Duration::from_secs(3),
            stream_window: 1024 * 1024,
            connection_window: 128 * 1024 * 1024,
            stream_budget_percent: 70,
            max_connections: 3,
            extra_idle: Duration::from_secs(60),
            poll_period: Duration::from_secs(1),
            // The longest pause plus a connect is below the recovery limit of 5 s.
            backoff: Backoff {
                first: Duration::from_millis(100),
                max: Duration::from_secs(3),
                jitter_percent: 50,
                reset_after: Duration::from_secs(2),
            },
        }
    }

    /// For lists, reads and writes. Measured: with a stream window of 1 MiB, a far server
    /// sends about 1 MiB for each round trip, and a list of 7 MiB is 7 times slower.
    pub fn for_requests() -> Self {
        Self { stream_window: 8 * 1024 * 1024, connection_window: 1024 * 1024 * 1024, ..Self::for_watches() }
    }

    /// For log streams.
    pub fn for_logs() -> Self {
        Self::for_watches()
    }

    /// The stream budget for a server limit (plan sections 8.6 and 8.7).
    ///
    /// The budget is also one below the count of stream windows that fit into the
    /// connection window. Thus bodies that nobody reads can fill each stream window, and the
    /// connection window still has room for one more stream: the test of the connection.
    pub(crate) fn budget(&self, server_limit: usize) -> usize {
        let by_limit = server_limit.saturating_mul(self.stream_budget_percent) / 100;
        let windows = self.connection_window / self.stream_window.max(1);
        let by_window = usize::try_from(windows).unwrap_or(usize::MAX).saturating_sub(1);
        by_limit.min(by_window).max(1)
    }
}

/// What the channels of one group carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Purpose {
    Watches,
    Requests,
    Logs,
}

impl Purpose {
    /// The name of a channel of this purpose, for the name of its task and for an error.
    fn channel(self) -> &'static str {
        match self {
            Self::Watches => "watch channel",
            Self::Requests => "request channel",
            Self::Logs => "logs channel",
        }
    }
}

/// What each channel of a cluster needs for its task.
#[derive(Clone)]
pub(crate) struct Parts {
    pub(crate) tasks: Tasks,
    /// The name of the cluster, for the names of the tasks.
    pub(crate) cluster: Arc<str>,
    pub(crate) dial: Arc<dyn Dial>,
    /// Builds each light request, and tells when the cluster has a token for one.
    pub(crate) light: Arc<dyn LightRequests>,
    pub(crate) signal: Signal,
    pub(crate) changed: Changed,
    /// The outage of the cluster, which each channel ends or extends with its connects.
    pub(crate) outage: Arc<Outage>,
}

/// The state that a channel shares with its task and with its requests.
pub(crate) struct Shared {
    purpose: Purpose,
    settings: ChannelSettings,
    /// An extra channel closes when it is idle. The first channel of a purpose stays.
    extra: bool,
    link: LinkBoard,
    pub(crate) streams: Arc<Streams>,
    status: StatusBoard,
    /// The number of the connection on which a request of a caller got an answer.
    answered: AtomicU64,
    answer_arrived: Notify,
}

/// One channel. A clone is one more handle to the same connection. The task of the channel
/// stops when the last handle drops.
#[derive(Clone)]
pub(crate) struct Channel {
    pub(crate) shared: Arc<Shared>,
    _task: Arc<TaskHandle>,
}

impl Channel {
    /// Starts the channel. It connects at once, and again after each loss. `budget` is the
    /// stream budget until the server of this connection sent its limit.
    pub(crate) fn start(
        parts: &Parts,
        purpose: Purpose,
        settings: &ChannelSettings,
        extra: bool,
        budget: usize,
        freed: Arc<Notify>,
    ) -> Self {
        let streams = Arc::new(Streams::new(budget, freed));
        let shared = Arc::new(Shared {
            purpose,
            settings: settings.clone(),
            extra,
            link: LinkBoard::new(),
            streams,
            status: StatusBoard::new(parts.changed.clone()),
            answered: AtomicU64::new(0),
            answer_arrived: Notify::new(),
        });
        let name = TaskName::scoped(purpose.channel(), Arc::clone(&parts.cluster));
        let body = {
            let (shared, parts) = (Arc::clone(&shared), parts.clone());
            move || driver::Driver::new(Arc::clone(&shared), &parts).run()
        };
        let task = parts.tasks.start(name, Restart::Always(settings.backoff), body);
        Self { shared, _task: Arc::new(task) }
    }

    /// Sends one request and waits for the head of its answer. The caller sets the time limit.
    ///
    /// One more try is safe only when hyper gives the request back: no byte of it was written.
    pub(crate) async fn send(&self, mut request: Request<Full<Bytes>>) -> Result<Response<Incoming>, NetError> {
        for _ in 0..2 {
            let (mut sender, generation) = self.shared.link.sender().await?;
            match sender.try_send_request(request).await {
                Ok(response) => {
                    self.shared.streams.frame_arrived();
                    self.shared.answered.store(generation, Ordering::Release);
                    self.shared.answer_arrived.notify_one();
                    return Ok(response);
                }
                Err(mut error) => match error.take_message() {
                    Some(unsent) => request = unsent,
                    None => return Err(NetError::because(self.shared.purpose.channel(), &error.into_error())),
                },
            }
        }
        let channel = self.shared.purpose.channel();
        Err(NetError::network(format!("{channel}: the connection closed two times before the request was written")))
    }

    pub(crate) fn status(&self) -> ChannelStatus {
        self.shared.status.read(&self.shared.streams)
    }
}

#[cfg(test)]
mod tests;
