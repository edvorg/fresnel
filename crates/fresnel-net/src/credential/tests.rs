//! The rules of the credential task and of the HTTP 401 layer, with paused time.
//!
//! A scripted token source stands for the plugin, and a scripted server gives the answers.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

use bytes::Bytes;
use futures_util::FutureExt as _;
use futures_util::future::join_all;
use http::header::AUTHORIZATION;
use http::{Request, Response, StatusCode};
use tokio::task::JoinSet;
use tokio::time::{Instant, sleep};

use super::document::Credential;
use super::*;
use crate::changes::Changed;
use crate::clock::testing::{epoch, paused_clock};
use crate::signal::Cause;

const SECOND: Duration = Duration::from_secs(1);
const STDERR: &str = "Error: access token is required (the secret text of the plugin)";

/// What one run of the scripted source gives. The last entry repeats.
#[derive(Clone, Copy)]
enum Run {
    /// A token, and the time to its expiry.
    Token(&'static str, Option<Duration>),
    Fails,
    Hangs,
}

struct Script {
    runs: Mutex<VecDeque<Run>>,
    clock: WallClock,
    in_progress: AtomicU64,
}

impl TokenSource for Script {
    fn run(&self, limit: Duration) -> BoxFuture<'_, Result<Credential, PluginError>> {
        async move {
            assert_eq!(self.in_progress.fetch_add(1, Ordering::SeqCst), 0, "two plugin runs at the same time");
            let run = {
                let mut runs = self.runs.lock().expect("a lock");
                if runs.len() > 1 { runs.pop_front() } else { runs.front().copied() }.expect("a scripted run")
            };
            // A run takes time, so that requests arrive while it is in progress.
            sleep(Duration::from_millis(10)).await;
            let result = match run {
                Run::Token(token, life) => Ok(Credential {
                    header: HeaderValue::from_str(&format!("Bearer {token}")).expect("a header"),
                    expires_at: life.map(|life| self.clock.now() + life),
                }),
                Run::Fails => {
                    Err(PluginError::new("fake", PluginFailure::Exit("exit status: 7".to_owned()), STDERR.to_owned()))
                }
                Run::Hangs => {
                    sleep(limit - Duration::from_millis(10)).await;
                    Err(PluginError::new("fake", PluginFailure::Timeout { limit, stopped: true }, String::new()))
                }
            };
            self.in_progress.fetch_sub(1, Ordering::SeqCst);
            result
        }
        .boxed()
    }
}

/// The scripted server: which tokens and which path it refuses.
#[derive(Default)]
struct Server {
    refused_tokens: Mutex<Vec<&'static str>>,
    refused_path: Mutex<Option<&'static str>>,
    silent_proof: AtomicBool,
    requests: AtomicU64,
}

impl Server {
    fn status(&self, path: &str, authorization: &HeaderValue) -> StatusCode {
        let token = authorization.to_str().expect("text").strip_prefix("Bearer ").expect("a bearer token");
        let bad_token =
            self.refused_tokens.lock().expect("a lock").iter().any(|refused| *refused == token || *refused == "*");
        let bad_path = self.refused_path.lock().expect("a lock").is_some_and(|prefix| path.starts_with(prefix));
        if bad_token || bad_path { StatusCode::UNAUTHORIZED } else { StatusCode::OK }
    }

    async fn answer(&self, request: Request<Bytes>) -> Result<Response<()>, NetError> {
        self.requests.fetch_add(1, Ordering::SeqCst);
        sleep(Duration::from_millis(5)).await;
        let authorization = request.headers().get(AUTHORIZATION).expect("each request has a token");
        let mut response = Response::new(());
        *response.status_mut() = self.status(request.uri().path(), authorization);
        Ok(response)
    }

    fn refuse_tokens(&self, tokens: &[&'static str]) {
        *self.refused_tokens.lock().expect("a lock") = tokens.to_vec();
    }
}

impl Prover for Server {
    fn prove(&self, authorization: HeaderValue) -> BoxFuture<'_, Proof> {
        async move {
            if self.silent_proof.load(Ordering::SeqCst) {
                sleep(3 * SECOND).await;
                return Proof::NoAnswer;
            }
            sleep(Duration::from_millis(5)).await;
            if self.status("/version", &authorization) == StatusCode::UNAUTHORIZED {
                Proof::Refused
            } else {
                Proof::Accepted
            }
        }
        .boxed()
    }
}

struct Bed {
    credentials: Credentials,
    server: Arc<Server>,
    signal: Signal,
    started: Instant,
    task: JoinSet<Result<(), NetError>>,
}

impl Bed {
    fn start(runs: &[Run]) -> Self {
        Self::with_clock(runs, paused_clock())
    }

    fn with_clock(runs: &[Run], clock: WallClock) -> Self {
        let credentials = Credentials::new(CredentialSettings::default(), clock.clone(), Changed::new());
        let source = Script { runs: Mutex::new(runs.iter().copied().collect()), clock, in_progress: AtomicU64::new(0) };
        let (server, signal) = (Arc::new(Server::default()), Signal::new());
        let mut task = JoinSet::new();
        task.spawn(credentials.task(Arc::new(source), Arc::clone(&server) as Arc<dyn Prover>, &signal));
        Self { credentials, server, signal, started: Instant::now(), task }
    }

    async fn get(&self, path: &'static str) -> Result<StatusCode, NetError> {
        let request = Request::get(path).body(Bytes::new()).expect("a request");
        let response = layer::send(&self.credentials, request, |request| self.server.answer(request)).await;
        response.map(|response| response.status())
    }

    /// The counts of plugin runs, of HTTP 401 answers and of proofs.
    fn counts(&self) -> (u64, u64, u64) {
        let status = self.credentials.status();
        (status.plugin_runs, status.unauthorized, status.proofs)
    }

    fn state(&self) -> CredentialState {
        self.credentials.status().state
    }

    fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }
}

#[tokio::test(start_paused = true)]
async fn the_plugin_runs_one_time_for_many_requests() {
    let bed = Bed::start(&[Run::Token("one", None)]);
    assert_eq!(bed.state(), CredentialState::None, "the task did not run yet");
    let answers = join_all((0..30).map(|_| bed.get("/api/v1/pods"))).await;
    assert!(answers.iter().all(|answer| answer == &Ok(StatusCode::OK)), "{answers:?}");
    assert_eq!(bed.counts(), (1, 0, 0));
    assert_eq!(bed.state(), CredentialState::Valid { until: None });
    assert_eq!(bed.credentials.authorization_now().expect("a token"), "Bearer one");
}

#[tokio::test(start_paused = true)]
async fn a_plugin_that_hangs_shows_a_timeout_and_a_request_waits_two_seconds() {
    let bed = Bed::start(&[Run::Hangs]);
    let error = bed.get("/api/v1/pods").await.expect_err("no token");
    assert_eq!(bed.elapsed(), 2 * SECOND, "the wait of a request has a limit");
    assert_eq!(
        (error.class(), error.to_string().as_str()),
        (ErrorClass::Auth, "no credential in 2s: the credential plugin runs")
    );
    assert!(matches!(bed.state(), CredentialState::Running { reason: RunReason::Start, last_failure: None, .. }));

    sleep(18 * SECOND + Duration::from_millis(1)).await;
    let CredentialState::Failed { failure, failures: 1, next_attempt } = bed.state() else {
        panic!("the state after the time limit of the plugin: {:?}", bed.state());
    };
    assert_eq!(failure.to_string(), "credential plugin `fake`: no answer in 20s");
    assert_eq!(next_attempt, epoch() + 22 * SECOND, "the next run starts after the first pause");
    let before = bed.elapsed();
    let error = bed.get("/api/v1/pods").await.expect_err("no token");
    assert_eq!((bed.elapsed(), error.to_string()), (before, failure.to_string()), "a failed state answers at once");
    assert_eq!(bed.server.requests.load(Ordering::SeqCst), 0, "no request went to the server");

    sleep(3 * SECOND).await;
    assert_eq!(bed.counts().0, 2);
    assert!(
        matches!(bed.state(), CredentialState::Running { reason: RunReason::Retry, last_failure: Some(last), .. } if last == failure),
        "the state keeps the failure during the next run: {:?}",
        bed.state()
    );
}

#[tokio::test(start_paused = true)]
async fn one_http_401_runs_the_plugin_one_time_and_each_request_goes_out_again() {
    let bed = Bed::start(&[Run::Token("one", None), Run::Token("two", None)]);
    bed.server.refuse_tokens(&["one"]);
    let answers = join_all((0..20).map(|_| bed.get("/api/v1/pods"))).await;
    assert!(answers.iter().all(|answer| answer == &Ok(StatusCode::OK)), "{answers:?}");
    assert_eq!(bed.counts(), (2, 20, 0), "20 refused requests cost one plugin run");
    assert_eq!(bed.get("/api/v1/pods").await, Ok(StatusCode::OK));
    assert_eq!(bed.counts(), (2, 20, 0));
}

#[tokio::test(start_paused = true)]
async fn an_http_401_of_one_path_fails_only_that_request() {
    let bed = Bed::start(&[Run::Token("one", None)]);
    *bed.server.refused_path.lock().expect("a lock") = Some("/apis/metrics.k8s.io/");
    let metrics = "/apis/metrics.k8s.io/v1beta1/nodes";

    let (refused, pods) = tokio::join!(bed.get(metrics), join_all((0..20).map(|_| bed.get("/api/v1/pods"))));
    assert_eq!(refused, Ok(StatusCode::UNAUTHORIZED), "the caller gets the answer of the server");
    assert!(pods.iter().all(|answer| answer == &Ok(StatusCode::OK)), "{pods:?}");
    assert_eq!(bed.counts(), (2, 2, 1), "one run after the first HTTP 401, one proof after the second");
    assert_eq!(bed.state(), CredentialState::Valid { until: None });

    // The sampler of the metrics reads this path again and again.
    for _ in 0..3 {
        sleep(9 * SECOND).await;
        let before = bed.elapsed();
        assert_eq!(bed.get(metrics).await, Ok(StatusCode::UNAUTHORIZED));
        assert_eq!(bed.elapsed() - before, Duration::from_millis(5), "one send, no wait and no second send");
        assert_eq!(bed.get("/api/v1/pods").await, Ok(StatusCode::OK));
    }
    assert_eq!(bed.counts(), (2, 5, 1), "a proven token starts no plugin run and no proof");

    sleep(10 * SECOND).await;
    assert_eq!(bed.get(metrics).await, Ok(StatusCode::UNAUTHORIZED));
    assert_eq!(bed.counts(), (2, 6, 2), "after its valid time, one more proof and no plugin run");
    assert_eq!(bed.state(), CredentialState::Valid { until: None });
}

#[tokio::test(start_paused = true)]
async fn a_server_that_refuses_each_token_gives_a_failed_state_with_backoff() {
    let bed = Bed::start(&[Run::Token("one", None)]);
    bed.server.refuse_tokens(&["*"]);
    let error = bed.get("/api/v1/pods").await.expect_err("a refused credential");
    assert_eq!(error.to_string(), "the server refused a new token of the credential plugin (HTTP 401)");
    assert_eq!(bed.counts(), (2, 2, 1));
    let refused_at = bed.elapsed();
    let failed = |failures, pause: Duration| CredentialState::Failed {
        failure: CredentialFailure::Refused,
        failures,
        next_attempt: epoch() + refused_at + pause,
    };
    assert_eq!(bed.state(), failed(1, 2 * SECOND));

    let sent = bed.server.requests.load(Ordering::SeqCst);
    assert!(bed.get("/api/v1/pods").await.is_err());
    assert_eq!(bed.server.requests.load(Ordering::SeqCst), sent, "no request goes out with a refused token");

    // The run after the pause gives a token, and the server refuses it too: the pause doubles.
    sleep(2 * SECOND + Duration::from_millis(20)).await;
    assert_eq!(bed.state(), CredentialState::Valid { until: None });
    assert!(bed.get("/api/v1/pods").await.is_err());
    assert_eq!(bed.counts(), (3, 3, 2), "a token that answers a refusal gets a proof and no second run");
    assert!(matches!(bed.state(), CredentialState::Failed { failure: CredentialFailure::Refused, failures: 2, .. }));

    // The server accepts tokens again. "Retry now" does not wait for the pause of 4 s.
    bed.server.refuse_tokens(&[]);
    bed.signal.raise(Cause::RetryNow);
    sleep(Duration::from_millis(1)).await;
    assert!(matches!(bed.state(), CredentialState::Running { reason: RunReason::Retry, .. }), "{:?}", bed.state());
    let before = bed.elapsed();
    assert_eq!(bed.get("/api/v1/pods").await, Ok(StatusCode::OK), "a request waits for a run in progress");
    assert!(bed.elapsed() - before < SECOND, "{:?}", bed.elapsed() - before);
    assert_eq!(bed.counts().0, 4);
}

#[tokio::test(start_paused = true)]
async fn a_failed_run_starts_again_after_its_pause_and_at_once_after_a_signal() {
    let bed = Bed::start(&[Run::Fails, Run::Fails, Run::Token("one", None)]);
    let error = bed.get("/api/v1/pods").await.expect_err("no token");
    assert_eq!(error.to_string(), "credential plugin `fake`: stopped with exit status: 7");
    let CredentialState::Failed { failure, .. } = bed.state() else { panic!("{:?}", bed.state()) };
    assert_eq!(failure.detail(), STDERR, "the UI gets the text of the plugin");
    for text in [error.to_string(), format!("{error:?}"), format!("{:?}", bed.state()), bed.state().to_string()] {
        assert!(!text.contains("secret"), "the text of the plugin is in a text that can go to a log: {text}");
    }

    sleep(2 * SECOND - Duration::from_millis(1)).await;
    assert_eq!(bed.counts().0, 1);
    sleep(Duration::from_millis(20)).await;
    assert_eq!(bed.counts().0, 2, "the second run starts after the pause of 2 s");
    assert!(matches!(bed.state(), CredentialState::Failed { failures: 2, .. }), "{:?}", bed.state());
    sleep(SECOND).await;
    bed.signal.raise(Cause::Wake);
    sleep(Duration::from_millis(20)).await;
    assert_eq!(
        (bed.counts().0, bed.state()),
        (3, CredentialState::Valid { until: None }),
        "no wait for the pause of 4 s"
    );
}

#[tokio::test(start_paused = true)]
async fn the_plugin_runs_again_before_the_expiry_and_a_plugin_with_a_cache_waits_for_it() {
    // The second run is 60 s before the expiry. The plugin gives its stored token again.
    let bed = Bed::start(&[
        Run::Token("one", Some(90 * SECOND)),
        Run::Token("one", Some(60 * SECOND)),
        Run::Token("two", None),
    ]);
    sleep(29 * SECOND).await;
    assert_eq!(
        (bed.counts().0, bed.state()),
        (1, CredentialState::Valid { until: Some(epoch() + 90 * SECOND + Duration::from_millis(10)) })
    );
    sleep(2 * SECOND).await;
    assert_eq!(bed.counts().0, 2, "the refresh starts one lead before the expiry");
    sleep(58 * SECOND).await;
    assert_eq!(bed.counts().0, 2, "a run before the expiry would give the stored token again");
    sleep(2 * SECOND).await;
    assert_eq!((bed.counts().0, bed.state()), (3, CredentialState::Valid { until: None }));
    assert_eq!(bed.credentials.authorization_now().expect("a token"), "Bearer two");
}

#[tokio::test(start_paused = true)]
async fn a_proof_with_no_answer_changes_nothing_and_a_stopped_task_shows_in_the_state() {
    let mut bed = Bed::start(&[Run::Token("one", None), Run::Token("two", None), Run::Hangs]);
    bed.server.refuse_tokens(&["one", "two"]);
    bed.server.silent_proof.store(true, Ordering::SeqCst);
    assert_eq!(bed.get("/api/v1/pods").await, Ok(StatusCode::UNAUTHORIZED), "nothing is known about the token");
    assert_eq!((bed.counts(), bed.state()), ((2, 2, 1), CredentialState::Valid { until: None }));

    // The token is not new now. The next HTTP 401 starts a run, and that run hangs.
    sleep(31 * SECOND).await;
    assert!(bed.get("/api/v1/pods").await.is_err());
    assert!(matches!(bed.state(), CredentialState::Running { reason: RunReason::Unauthorized, .. }));
    bed.task.abort_all();
    assert!(bed.task.join_next().await.expect("one task").expect_err("an abort").is_cancelled());
    assert!(
        matches!(bed.state(), CredentialState::Failed { failure: CredentialFailure::Stopped, .. }),
        "a run that nobody waits for must not show: {:?}",
        bed.state()
    );
}

/// The monotonic clock stops while the laptop sleeps, and the token expires by the wall clock.
#[tokio::test(start_paused = true)]
async fn after_a_wake_the_task_checks_the_expiry_time_at_once() {
    let (start, slept) = (Instant::now(), Arc::new(AtomicU64::new(0)));
    let clock = {
        let slept = Arc::clone(&slept);
        WallClock::new(move || epoch() + start.elapsed() + Duration::from_secs(slept.load(Ordering::SeqCst)))
    };
    let bed = Bed::with_clock(&[Run::Token("one", Some(3600 * SECOND)), Run::Token("two", None)], clock);
    sleep(10 * SECOND).await;
    assert_eq!(bed.counts().0, 1);

    // The laptop sleeps for 2 hours. Only the wall clock moves.
    slept.store(7200, Ordering::SeqCst);
    bed.signal.raise(Cause::Wake);
    sleep(Duration::from_millis(20)).await;
    assert_eq!(bed.counts().0, 2, "the token expired during the sleep, so the plugin runs at the wake");
    assert_eq!(bed.credentials.authorization_now().expect("a token"), "Bearer two");
}
