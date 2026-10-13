//! The channel against a server in memory, with paused time. Each time here is exact.

use std::sync::Mutex;
use std::time::Duration;

use http::Request;
use http_body_util::BodyExt as _;
use tokio::runtime::Handle;
use tokio::time::{Instant, sleep, timeout};

use super::group::Group;
use super::*;
use crate::changes::Changes;
use crate::error::ErrorClass;
use crate::signal::Cause;
use crate::task::TaskReport;
use crate::testing::{Connects, MemoryServer};

mod live;
mod windows;

const SECOND: Duration = Duration::from_secs(1);
const MS: Duration = Duration::from_millis(1);

struct Bed {
    server: Arc<MemoryServer>,
    group: Group,
    signal: Signal,
    changes: Changes,
    started: Instant,
}

impl Bed {
    fn start(settings: ChannelSettings) -> Self {
        Self::on(MemoryServer::new(), settings)
    }

    fn on(server: Arc<MemoryServer>, settings: ChannelSettings) -> Self {
        let parts = server.parts();
        let (signal, changes) = (parts.signal.clone(), parts.changed.watch());
        Self {
            server,
            group: Group::start(parts, Purpose::Watches, settings),
            signal,
            changes,
            started: Instant::now(),
        }
    }

    /// The status of the first channel.
    fn status(&self) -> ChannelStatus {
        self.group.status().channels.remove(0)
    }

    /// Waits until the first channel meets the condition. The limit is in paused time.
    async fn until(&mut self, what: &str, condition: impl Fn(&ChannelStatus) -> bool) -> ChannelStatus {
        let wait = async {
            loop {
                let status = self.group.status().channels.remove(0);
                if condition(&status) {
                    return status;
                }
                self.changes.next().await;
            }
        };
        match timeout(120 * SECOND, wait).await {
            Ok(status) => status,
            Err(_) => panic!("no state `{what}` in time: {:?}", self.status()),
        }
    }

    async fn until_state(&mut self, state: ChannelState) -> ChannelStatus {
        let text = format!("{state:?}");
        self.until(&text, |status| status.state == state).await
    }

    /// Sends one request and gives the answer with its stream.
    async fn open(&self, path: &'static str) -> Result<(Response<Incoming>, streams::StreamPermit), NetError> {
        let request = Request::get(format!("https://memory{path}")).body(Full::default()).expect("a request");
        let (channel, stream) = self.group.stream().await;
        Ok((channel.send(request).await?, stream))
    }

    fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }
}

fn settings() -> ChannelSettings {
    ChannelSettings::for_watches()
}

#[tokio::test(start_paused = true)]
async fn a_silent_connection_shows_after_15_seconds_with_a_stream_and_with_none() {
    for with_stream in [true, false] {
        let mut bed = Bed::start(settings());
        bed.until_state(ChannelState::Live).await;
        let stream = if with_stream { Some(bed.open("/watch/pods").await.expect("a stream")) } else { None };
        sleep(4 * SECOND).await;
        bed.server.silence_open();
        let silent_at = bed.elapsed();

        // The channel is live again in the same instant, so the test waits for the record of the loss.
        let status = bed.until("the loss of the connection", |status| status.last_error.is_some()).await;
        let detected_after = bed.elapsed() - silent_at;
        // The last frame was 4 s before the fault: ping after 10 s with no frame, close 5 s later.
        assert_eq!(detected_after, 11 * SECOND, "with a stream: {with_stream}");
        assert!(detected_after <= 20 * SECOND, "the limit of milestone M0");
        let error = status.last_error.expect("an error");
        assert!(error.starts_with("http2 error: keep-alive timed out"), "{error}");

        let live =
            bed.until("live again", |status| status.state == ChannelState::Live && status.connections == 2).await;
        assert_eq!(bed.elapsed() - silent_at, 11 * SECOND, "the channel connects again by itself, with no pause");
        assert_eq!((live.connect_attempts, bed.server.dials()), (2, 2));
        if let Some((answer, _stream)) = stream {
            let ended = answer.into_body().frame().await.expect("the end of the stream");
            assert!(ended.is_err(), "a stream of the silent connection ends with an error");
        }
    }
}

#[tokio::test(start_paused = true)]
async fn a_wake_tests_the_connection_with_a_limit_of_3_seconds() {
    let mut bed = Bed::start(settings());
    bed.until_state(ChannelState::Live).await;
    let _stream = bed.open("/watch/pods").await.expect("a stream");
    sleep(4 * SECOND).await;
    bed.signal.raise(Cause::Wake);
    sleep(SECOND).await;
    assert_eq!((bed.status().connections, bed.server.seen().len()), (1, 3), "a good connection passes the test");

    bed.server.silence_open();
    let raised_at = bed.elapsed();
    bed.signal.raise(Cause::NetworkChange);
    let status = bed.until("the loss of the connection", |status| status.last_error.is_some()).await;
    assert_eq!(bed.elapsed() - raised_at, 3 * SECOND, "the test does not wait for the ping");
    assert_eq!(status.last_error.as_deref(), Some("no answer to the light request in 3s"));
    bed.until("live again", |status| status.state == ChannelState::Live && status.connections == 2).await;
    assert_eq!(bed.elapsed() - raised_at, 3 * SECOND, "a new connection follows at once");
}

#[tokio::test(start_paused = true)]
async fn an_idle_channel_sends_a_light_request_each_30_seconds() {
    let mut bed = Bed::start(settings());
    bed.until_state(ChannelState::Live).await;
    sleep(100 * SECOND).await;
    let paths: Vec<String> = bed.server.seen().into_iter().map(|seen| seen.path).collect();
    assert_eq!(paths, ["/version"; 4], "the first request and one each 30 s");
    assert_eq!(bed.server.dials(), 1);

    // A channel with an open stream is in use. The server does not close it.
    let (_answer, _stream) = bed.open("/watch/pods").await.expect("a stream");
    sleep(100 * SECOND).await;
    assert_eq!(bed.server.seen().len(), 5, "no light request while a stream is open");
    assert_eq!((bed.status().state, bed.server.dials()), (ChannelState::Live, 1));
}

#[tokio::test(start_paused = true)]
async fn each_request_waits_for_the_one_result_of_the_connect() {
    let server = MemoryServer::new();
    server.set_connects(Connects::Held);
    let bed = Bed::on(server, settings());
    let requests = futures_util::future::join_all((0..5).map(|_| bed.open("/api/v1/pods")));
    let release = async {
        sleep(4 * SECOND).await;
        bed.server.set_connects(Connects::Refused);
    };
    let (answers, ()) = tokio::join!(requests, release);
    assert_eq!(bed.elapsed(), 4 * SECOND);
    for answer in answers {
        let error = answer.err().expect("the connect failed");
        assert_eq!((error.class(), error.to_string().as_str()), (ErrorClass::Network, "connection refused"));
    }
    assert_eq!(bed.status().connect_attempts, 1, "one connect for five requests");

    // A request that comes during the pause waits for the next connect, not for the old error.
    bed.server.set_connects(Connects::Work);
    assert!(bed.open("/api/v1/pods").await.is_ok());
    assert_eq!((bed.status().connect_attempts, bed.server.dials()), (2, 1));
}

#[tokio::test(start_paused = true)]
async fn failed_connects_wait_longer_each_time_and_the_signal_ends_the_wait() {
    let server = MemoryServer::new();
    server.set_connects(Connects::Refused);
    let mut bed = Bed::on(server, settings());
    let mut pauses = Vec::new();
    for failures in 1..=7 {
        let status = bed
            .until(
                "backoff",
                |status| matches!(status.state, ChannelState::Backoff { failures: now, .. } if now == failures),
            )
            .await;
        let ChannelState::Backoff { pause, .. } = status.state else { unreachable!() };
        pauses.push(pause);
    }
    for (pause, full) in pauses.iter().zip([100, 200, 400, 800, 1600, 3000, 3000]) {
        assert!((full * MS / 2..=full * MS).contains(pause), "{pauses:?}");
    }
    assert!(bed.elapsed() <= pauses.iter().sum::<Duration>(), "each connect starts when its pause ends");

    // The pause of 1.5 to 3 s is in progress. The signal starts the connect now.
    bed.server.set_connects(Connects::Work);
    let before = bed.elapsed();
    bed.signal.raise(Cause::RetryNow);
    bed.until_state(ChannelState::Live).await;
    assert_eq!(bed.elapsed(), before);
}

/// A connect that started before the network returned can wait on a dead path.
#[tokio::test(start_paused = true)]
async fn a_signal_starts_a_connect_in_progress_again() {
    let mut bed = Bed::start(settings());
    bed.until_state(ChannelState::Live).await;
    sleep(3 * SECOND).await;
    bed.server.set_connects(Connects::Lost);
    bed.server.kill_open();
    bed.until("a connect in progress", |status| status.connect_attempts == 2).await;
    bed.server.set_connects(Connects::Work);
    sleep(5 * SECOND).await;
    assert_eq!(bed.status().state, ChannelState::Connecting { attempt: 1 }, "the lost connect is still in progress");

    bed.signal.raise(Cause::NetworkChange);
    let live = bed.until_state(ChannelState::Live).await;
    assert_eq!((bed.elapsed(), live.connections, live.connect_attempts), (8 * SECOND, 2, 2));
}

#[tokio::test(start_paused = true)]
async fn eight_resets_in_one_minute_each_recover_at_once() {
    let mut bed = Bed::start(settings());
    bed.until_state(ChannelState::Live).await;
    for reset in 1..=8_u64 {
        sleep(7 * SECOND).await;
        let (answer, _stream) = bed.open("/watch/pods").await.expect("a stream");
        bed.server.kill_open();
        let killed_at = bed.elapsed();
        assert!(answer.into_body().frame().await.expect("the end of the stream").is_err());
        let live = bed
            .until("live again", |status| status.state == ChannelState::Live && status.connections == reset + 1)
            .await;
        assert_eq!(bed.elapsed() - killed_at, Duration::ZERO, "reset {reset}: a loss counts one time and has no pause");
        assert_eq!(live.connect_attempts, reset + 1);
        assert!(bed.open("/api/v1/pods").await.is_ok());
    }
    assert!(bed.elapsed() < 60 * SECOND);
}

#[tokio::test(start_paused = true)]
async fn a_connection_that_is_lost_early_again_and_again_gets_a_pause_below_the_limit() {
    let mut bed = Bed::start(settings());
    for connection in 1..=12_u64 {
        bed.until("live", |status| status.state == ChannelState::Live && status.connections == connection).await;
        let live_at = bed.elapsed();
        // The connection is lost before it counts as stable.
        sleep(100 * MS).await;
        bed.server.kill_open();
        bed.until("live again", |status| status.state == ChannelState::Live && status.connections == connection + 1)
            .await;
        let recovery = bed.elapsed() - live_at - 100 * MS;
        assert!(recovery <= 3 * SECOND, "connection {connection}: {recovery:?}");
        assert!(connection < 6 || recovery >= 1500 * MS, "a server that closes each connection gets no busy loop");
    }
}

#[tokio::test(start_paused = true)]
async fn after_goaway_the_channel_connects_again_and_open_streams_continue() {
    let mut bed = Bed::start(settings());
    bed.until_state(ChannelState::Live).await;
    let (answer, _stream) = bed.open("/watch/pods").await.expect("a stream");
    let mut events = answer.into_body();
    bed.server.go_away();
    let gone_at = bed.elapsed();

    let live =
        bed.until("a new connection", |status| status.state == ChannelState::Live && status.connections == 2).await;
    assert!(
        bed.elapsed() - gone_at <= SECOND,
        "the channel looks for GOAWAY each second: {:?}",
        bed.elapsed() - gone_at
    );
    assert_eq!(live.streams_open, 1, "the stream of the old connection still counts");

    bed.server.send_event("event after GOAWAY\n");
    let event = events.frame().await.expect("a frame").expect("the old connection works").into_data().expect("data");
    assert_eq!(event, "event after GOAWAY\n");
    assert!(bed.open("/api/v1/pods").await.is_ok());
    let last = bed.server.seen().pop().expect("a request");
    assert_eq!((last.connection, last.path.as_str()), (1, "/api/v1/pods"), "a new request uses the new connection");
}

#[tokio::test(start_paused = true)]
async fn a_request_right_after_goaway_goes_to_the_new_connection_with_no_error() {
    let mut bed = Bed::start(settings());
    bed.until_state(ChannelState::Live).await;
    let _stream = bed.open("/watch/pods").await.expect("a stream");
    sleep(3 * SECOND).await;
    bed.server.go_away();
    sleep(MS).await;
    assert!(bed.open("/api/v1/pods").await.is_ok(), "hyper gives the request back, and the channel sends it again");
    assert_eq!((bed.elapsed(), bed.server.dials()), (3 * SECOND + MS, 2), "no wait for the next look of the channel");
}

/// A defect in the task of a channel must not leave a state that looks good (design rule 1).
#[tokio::test(start_paused = true)]
async fn a_panic_in_the_task_of_a_channel_gives_a_report_and_the_channel_connects_again() {
    let server = MemoryServer::new();
    let mut parts = server.parts();
    let reports = Arc::new(Mutex::new(Vec::new()));
    let kept = Arc::clone(&reports);
    parts.tasks =
        Tasks::new(Handle::current(), move |report: TaskReport| kept.lock().expect("a lock").push(report.to_string()));
    // The task builds a light request at its first connect and after 30 s. The second one fails.
    let (built, build) = (AtomicU64::new(0), Arc::clone(&parts.light_request));
    parts.light_request = Arc::new(move |protocol| {
        assert_ne!(built.fetch_add(1, Ordering::SeqCst), 1, "a defect in the task of the channel");
        build(protocol)
    });
    let mut changes = parts.changed.watch();
    let group = Group::start(parts, Purpose::Watches, settings());
    let status = || group.status().channels.remove(0);

    while status().last_error.is_none() {
        timeout(60 * SECOND, changes.next()).await.expect("the panic in time");
    }
    assert_eq!(status().last_error.as_deref(), Some("the task of the channel ended"));
    assert_ne!(status().state, ChannelState::Live, "the state must not show the lost connection as live");
    let report = reports.lock().expect("a lock").join("; ");
    assert!(
        report.starts_with("task watch channel [memory] panicked: assertion `left != right` failed: a defect"),
        "{report}"
    );

    while !(status().state == ChannelState::Live && status().connections == 2) {
        timeout(60 * SECOND, changes.next()).await.expect("a new connection in time");
    }
    let request = Request::get("https://memory/api/v1/pods").body(Full::default()).expect("a request");
    let (channel, _stream) = group.stream().await;
    assert!(channel.send(request).await.is_ok(), "the channel works after the restart of its task");
}
