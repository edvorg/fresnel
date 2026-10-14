//! The task of one channel: connect, prove the connection, serve it, and connect again.
//!
//! The task connects by itself after each loss. It does not wait for a request. A loss
//! counts one time, here, and not in each watch that used the connection. Thus the pause
//! before a connect does not grow with the count of watches. In a long outage, the
//! [`Outage`] of the cluster sets the pause.

use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime};

use futures_util::StreamExt as _;
use futures_util::future::BoxFuture;
use futures_util::stream::FuturesUnordered;
use http::StatusCode;
use http::header::AUTHORIZATION;
use http_body_util::BodyExt as _;
use tokio::time::{Instant, MissedTickBehavior, interval_at, timeout};

use super::connection::{Connection, End};
use super::link::Sender;
use super::outage::Outage;
use super::state::ChannelState;
use super::{Parts, Shared};
use crate::connect::dial::Protocol;
use crate::connect::{Dial, Effort};
use crate::error::NetError;
use crate::light::LightRequests;
use crate::signal::Listener;

/// Old connections with open streams that the task keeps after GOAWAY. More than this is
/// a server that sends GOAWAY again and again. The streams of each old connection then
/// end, and only the newest old connection stays.
const MAX_DRAINING: usize = 4;

/// The text of the end of a connection after HTTP 401 to its light request.
const REFUSED: &str = "the server refused the light request (HTTP 401)";

/// One light request in progress: its answer, or why it got none.
type Check = Pin<Box<dyn Future<Output = Result<Answer, NetError>> + Send>>;

/// What the server said to a light request.
enum Answer {
    /// The connection and the server work. The value is the round trip time.
    Accepted(Duration),
    /// HTTP 401. The check gave it to the credential of the cluster already.
    Refused,
}

pub(super) struct Driver {
    shared: Arc<Shared>,
    dial: Arc<dyn Dial>,
    light: Arc<dyn LightRequests>,
    listener: Listener,
    outage: Arc<Outage>,
    /// Failures in sequence: connects that failed and connections that were lost early.
    failures: u32,
    /// Counts the connections of the channel.
    generation: u64,
}

/// Why the serve of one connection ended.
enum Ended {
    /// A clean end by the server. Open streams continue on the old connection, and the
    /// light request in progress is one of them.
    Closed(Option<Check>),
    /// HTTP 401 to the light request. A kube-apiserver then ends the connection in the
    /// clean way, so the channel does the same and does not wait for it.
    Refused,
    Failed(String),
    /// The channel is an extra one and had no stream for some time.
    Retired,
}

/// A run that ends, for example with a panic, takes its connection with it. The state must
/// not show that connection as live until the start function runs the task again.
impl Drop for Driver {
    fn drop(&mut self) {
        self.shared.link.lost();
        self.shared.status.write(|facts| {
            facts.state = ChannelState::Connecting { attempt: 1 };
            facts.since = SystemTime::now();
            facts.last_error = Some("the task of the channel ended".to_owned());
        });
    }
}

impl Driver {
    pub(super) fn new(shared: Arc<Shared>, parts: &Parts) -> Self {
        let generation = shared.status.connections();
        Self {
            shared,
            dial: Arc::clone(&parts.dial),
            light: Arc::clone(&parts.light),
            listener: parts.signal.listen(),
            outage: Arc::clone(&parts.outage),
            failures: 0,
            generation,
        }
    }

    /// Runs until the handle of the channel drops. The start function runs it again after a
    /// panic, so the run starts from a clean state.
    pub(super) async fn run(mut self) -> Result<(), NetError> {
        self.shared.link.lost();
        let mut draining = Draining::default();
        let mut effort = Effort::Race;
        loop {
            if self.shared.streams.is_retired() {
                self.shared.status.enter(ChannelState::Retired);
                return std::future::pending().await;
            }
            let (sender, mut connection) = match draining.during(self.connect(effort)).await {
                Ok(connected) => connected,
                Err(error) => {
                    self.shared.link.connect_failed(error.clone());
                    effort = draining.during(self.pause_after(error.to_string(), false)).await;
                    continue;
                }
            };
            let (ended, stable) = draining.during(self.serve(sender, &mut connection)).await;
            self.shared.link.lost();
            let reason = match ended {
                Ended::Retired => continue,
                Ended::Failed(reason) => {
                    // The drop ends each stream of the connection before the pause.
                    drop(connection);
                    reason
                }
                Ended::Closed(check) => {
                    draining.add(connection, check);
                    "the server closed the connection".to_owned()
                }
                Ended::Refused => {
                    draining.add(connection, None);
                    REFUSED.to_owned()
                }
            };
            effort = draining.during(self.pause_after(reason, stable)).await;
        }
    }

    /// One connect, which the outage of the cluster counts. A round of the signal starts it
    /// again as a race: a connect that started before can wait on a dead path, and a new one
    /// is faster.
    async fn connect(&mut self, mut effort: Effort) -> Result<(Sender, Connection), NetError> {
        let attempt = self.failures.saturating_add(1);
        self.shared.status.write(|facts| {
            facts.state = ChannelState::Connecting { attempt };
            facts.since = SystemTime::now();
            facts.connect_attempts += 1;
        });
        let started = Instant::now();
        let dialed = loop {
            tokio::select! {
                connected = self.dial.dial(Protocol::Http2, effort) => break connected,
                () = self.outage.round(&mut self.listener) => effort = Effort::Race,
            }
        };
        let connected = match dialed {
            Ok(io) => Connection::handshake(io, &self.shared.settings).await,
            Err(error) => Err(error),
        };
        match &connected {
            Ok(_) => self.outage.worked(),
            Err(_) => self.outage.failed(started),
        }
        connected
    }

    /// Records the end of a connect or of a connection, waits before the next connect and
    /// gives its effort. After a connection that was live for some time, the next connect
    /// starts at once.
    async fn pause_after(&mut self, reason: String, stable: bool) -> Effort {
        self.shared.status.write(|facts| facts.last_error = Some(reason));
        if stable {
            self.failures = 0;
            return Effort::Race;
        }
        self.failures = self.failures.saturating_add(1);
        let (failures, status) = (self.failures, &self.shared.status);
        let own = self.shared.settings.backoff.pause(failures, fastrand::u32(..));
        // A round of the signal and a channel that connects end the pause.
        let show = |pause| status.enter(ChannelState::Backoff { failures, pause });
        self.outage.pause(&mut self.listener, own, show).await
    }

    /// Serves one connection to its end. Gives the end, and whether the connection was live
    /// for long enough to count as stable.
    async fn serve(&mut self, sender: Sender, connection: &mut Connection) -> (Ended, bool) {
        let settings = self.shared.settings.clone();
        self.generation += 1;
        let generation = self.generation;
        self.shared.link.up(sender.clone(), generation);
        // A server that starts can accept a connection and then close it. Only an answer
        // shows that the connection works. With no token, the light request waits.
        let mut check = self.check(&sender, settings.answer_limit);
        let state = if check.is_some() { ChannelState::Verifying } else { ChannelState::WaitsForToken };
        self.shared.status.write(|facts| {
            facts.state = state;
            facts.since = SystemTime::now();
            facts.connections = generation;
        });
        let mut live_since = None;
        let mut idle_since = Instant::now();
        let mut poll = interval_at(Instant::now() + settings.poll_period, settings.poll_period);
        poll.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let ended = loop {
            tokio::select! {
                end = connection.run() => break match end {
                    End::Closed => Ended::Closed(check.take()),
                    End::Failed(reason) => Ended::Failed(reason),
                },
                answer = in_progress(&mut check) => {
                    check = None;
                    match answer {
                        Ok(Answer::Accepted(round_trip)) => {
                            self.shared.streams.frame_arrived();
                            self.shared.status.write(|facts| facts.ping_time = Some(round_trip));
                            self.go_live(&mut live_since, connection);
                        }
                        Ok(Answer::Refused) => break Ended::Refused,
                        Err(error) => break Ended::Failed(error.to_string()),
                    }
                }
                // A request of a caller got its answer first.
                () = self.shared.answer_arrived.notified(), if live_since.is_none() => {
                    if self.shared.answered.load(Ordering::Acquire) == generation {
                        self.go_live(&mut live_since, connection);
                    }
                }
                // The token arrived, so the light request of this connection can go out.
                () = self.light.token(), if check.is_none() && live_since.is_none() => {
                    check = self.check(&sender, settings.answer_limit);
                    if check.is_some() {
                        self.shared.status.enter(ChannelState::Verifying);
                    }
                }
                // After a wake or a network change, an open connection can be silent. With
                // no token, only the ping tests it.
                _cause = self.listener.raised() => {
                    if let Some(test) = self.check(&sender, settings.test_limit) {
                        check = Some(test);
                    }
                }
                _ = poll.tick() => {
                    if live_since.is_some() {
                        self.read_stream_limit(connection);
                    }
                    if self.shared.streams.open() > 0 {
                        idle_since = Instant::now();
                    } else if self.shared.extra {
                        if idle_since.elapsed() >= settings.extra_idle && self.shared.streams.retire() {
                            break Ended::Retired;
                        }
                    } else if check.is_none() && self.light_request_is_due() {
                        check = self.check(&sender, settings.answer_limit);
                    }
                }
            }
        };
        let stable = live_since.is_some_and(|since: Instant| since.elapsed() >= settings.backoff.reset_after);
        (ended, stable)
    }

    fn go_live(&self, live_since: &mut Option<Instant>, connection: &Connection) {
        if live_since.is_none() {
            *live_since = Some(Instant::now());
            // The settings of the server arrive before its first answer.
            self.read_stream_limit(connection);
            self.shared.status.enter(ChannelState::Live);
        }
    }

    fn read_stream_limit(&self, connection: &Connection) {
        if let Some(limit) = connection.stream_limit() {
            self.shared.streams.set_limit(limit, self.shared.settings.budget(limit));
        }
    }

    /// A server closes a connection with no request for some time. The ping does not count
    /// for it, so an idle channel sends a small request.
    fn light_request_is_due(&self) -> bool {
        self.shared.streams.last_frame_age().is_none_or(|age| age >= self.shared.settings.light_period)
    }

    /// Sends the light request and reads its full answer, so that its stream ends in the
    /// normal way. `None` while the cluster has no token: no request goes out then.
    ///
    /// HTTP 401 goes to the credential of the cluster. Without that report, a token that the
    /// server refuses would cost one connection for each light request, with no visible cause.
    fn check(&self, sender: &Sender, limit: Duration) -> Option<Check> {
        let (mut sender, request) = (sender.clone(), self.light.build(Protocol::Http2)?);
        let (light, token) = (Arc::clone(&self.light), request.headers().get(AUTHORIZATION).cloned());
        Some(Box::pin(async move {
            let started = Instant::now();
            let exchange = async {
                let (head, mut body) = sender.send_request(request).await?.into_parts();
                while let Some(frame) = body.frame().await {
                    frame?;
                }
                Ok::<_, hyper::Error>(head.status)
            };
            match timeout(limit, exchange).await {
                Ok(Ok(StatusCode::UNAUTHORIZED)) => {
                    if let Some(token) = &token {
                        light.refused(token);
                    }
                    Ok(Answer::Refused)
                }
                Ok(Ok(_)) => Ok(Answer::Accepted(started.elapsed())),
                Ok(Err(error)) => Err(NetError::because("the light request failed", &error)),
                Err(_) => Err(NetError::timeout(format!("no answer to the light request in {limit:?}"))),
            }
        }))
    }
}

/// Waits for the light request in progress. With none, it waits with no end.
async fn in_progress(check: &mut Option<Check>) -> Result<Answer, NetError> {
    match check.as_mut() {
        Some(check) => check.await,
        None => std::future::pending().await,
    }
}

/// The old connections that still have open streams after GOAWAY.
#[derive(Default)]
struct Draining(FuturesUnordered<BoxFuture<'static, ()>>);

impl Draining {
    /// Keeps an old connection until its last stream ends. `check` is the light request
    /// that was in progress on it: a kube-apiserver sends HTTP 401 and GOAWAY together, so
    /// the answer is read here, and an HTTP 401 still reaches the credential.
    fn add(&mut self, connection: Connection, check: Option<Check>) {
        if self.0.len() >= MAX_DRAINING {
            self.0.clear();
        }
        let answer = async {
            let Some(check) = check else { return };
            match check.await {
                // The check gave an HTTP 401 to the credential. The connection is closed in
                // each case, so no other result changes something.
                Ok(Answer::Accepted(_) | Answer::Refused) | Err(_) => {}
            }
        };
        self.0.push(Box::pin(async move {
            tokio::join!(connection.drain(), answer);
        }));
    }

    /// Runs `work` and, at the same time, the old connections.
    async fn during<T>(&mut self, work: impl Future<Output = T>) -> T {
        tokio::pin!(work);
        loop {
            tokio::select! {
                output = &mut work => return output,
                Some(()) = self.0.next() => {}
            }
        }
    }
}
