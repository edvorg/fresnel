//! One request through each step of the layer, against a server in memory with paused time.

use std::future::poll_fn;
use std::sync::Mutex;
use std::time::Duration;

use http::header::{ACCEPT_ENCODING, AUTHORIZATION, CONTENT_ENCODING};
use http::{Request, StatusCode};
use http_body_util::BodyExt as _;
use rustls::{ClientConfig, RootCertStore};
use tokio::runtime::Handle;
use tokio::time::{Instant, sleep, timeout};
use tower_service::Service as _;

use super::*;
use crate::channel::ChannelState;
use crate::clock::testing::{epoch, paused_clock};
use crate::credential::CredentialState;
use crate::error::ErrorClass;
use crate::request::RequestKind;
use crate::task::TaskReport;
use crate::testing::{Connects, MemoryServer, Tokens, big_text};

const SECOND: Duration = Duration::from_secs(1);
const METRICS: &str = "/apis/metrics.k8s.io/v1beta1/nodes";

struct Bed {
    server: Arc<MemoryServer>,
    cluster: Cluster,
    started: Instant,
    /// One line for each run of a task that ended. A test that expects none reads nothing,
    /// and the drop of the bed checks the list.
    reports: Arc<Mutex<Vec<String>>>,
}

impl Drop for Bed {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            assert_eq!(*self.reports.lock().expect("a lock"), Vec::<String>::new(), "a task of the cluster ended");
        }
    }
}

impl Bed {
    /// A cluster on a server in memory. With tokens, the cluster has a credential.
    async fn start(tokens: Option<&[&'static str]>) -> Self {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let tls =
            ClientConfig::builder_with_provider(provider).with_safe_default_protocol_versions().expect("versions");
        let tls = tls.with_root_certificates(RootCertStore::empty()).with_no_client_auth();
        let endpoint = Endpoint {
            host: "memory".to_owned(),
            port: 6443,
            base_path: String::new(),
            tls_server_name: None,
            tls: Arc::new(tls),
        };
        let address = Address::new(&endpoint, "fresnel/test", "/version").expect("an address");
        let server = MemoryServer::new();
        let reports = Arc::new(Mutex::new(Vec::new()));
        let kept = Arc::clone(&reports);
        let tasks = Tasks::new(Handle::current(), move |report: TaskReport| {
            kept.lock().expect("a lock").push(report.to_string())
        });
        let source = tokens.map(|tokens| Tokens::new(tokens) as Arc<dyn TokenSource>);
        let dial = Arc::clone(&server) as Arc<dyn Dial>;
        let cluster =
            Cluster::build(&tasks, "memory".into(), address, dial, source, NetSettings::default(), paused_clock());
        let mut bed = Self { server, cluster, started: Instant::now(), reports };
        bed.until("each channel is live", |status| {
            let groups = [&status.watches, &status.requests, &status.logs];
            let ready = status.credential.as_ref().is_none_or(|credential| credential.plugin_runs > 0);
            ready && groups.iter().all(|group| group.channels.iter().all(|channel| channel.state == ChannelState::Live))
        })
        .await;
        bed.started = Instant::now();
        bed
    }

    async fn until(&mut self, what: &str, condition: impl Fn(&ClusterStatus) -> bool) -> ClusterStatus {
        let mut changes = self.cluster.changes();
        let wait = async {
            loop {
                let status = self.cluster.status();
                if condition(&status) {
                    return status;
                }
                changes.next().await;
            }
        };
        timeout(120 * SECOND, wait).await.unwrap_or_else(|_| panic!("no state `{what}`: {:?}", self.cluster.status()))
    }

    async fn send(&self, kind: RequestKind, path: &str) -> Result<Response<ResponseBody>, NetError> {
        let mut request = Request::get(path).body(Bytes::new()).expect("a request");
        request.extensions_mut().insert(kind);
        self.cluster.send(request).await
    }

    /// Sends a request and reads its full answer. No read of the layer is longer than 120 s.
    async fn fetch(&self, kind: RequestKind, path: &str) -> Result<(StatusCode, String), NetError> {
        let (head, body) = self.send(kind, path).await?.into_parts();
        let text =
            timeout(200 * SECOND, body.collect()).await.expect("a time limit of the layer ends the read")?.to_bytes();
        Ok((head.status, String::from_utf8(text.to_vec()).expect("text")))
    }

    fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// The requests that the server saw, with no light request.
    fn seen(&self) -> Vec<crate::testing::Seen> {
        self.server.seen().into_iter().filter(|seen| seen.path != "/version").collect()
    }
}

#[tokio::test(start_paused = true)]
async fn each_kind_fails_at_its_time_limit_when_the_server_gives_no_answer() {
    let bed = Bed::start(None).await;
    bed.server.set_mute(true);
    bed.server.set_connects(Connects::Held);
    let kinds = [
        (RequestKind::Get, 10),
        (RequestKind::ListPage, 15),
        (RequestKind::Watch, 10),
        (RequestKind::LogStream, 10),
        (RequestKind::Write, 15),
        (RequestKind::Upgrade, 10),
    ];
    let bed = &bed;
    let failed = futures_util::future::join_all(kinds.map(|(kind, seconds)| async move {
        let error = bed.send(kind, "/api/v1/pods").await.expect_err("no answer");
        assert_eq!(bed.elapsed(), seconds * SECOND, "{kind:?}");
        assert_eq!(
            (error.class(), error.to_string()),
            (ErrorClass::Timeout, format!("{kind:?} request: no answer in {seconds}s"))
        );
    }));
    failed.await;
    let status = bed.cluster.status();
    let open = [&status.watches, &status.requests, &status.logs].map(|group| group.channels[0].streams_open);
    assert_eq!(open, [0, 0, 0], "a request that failed holds no stream");
}

#[tokio::test(start_paused = true)]
async fn the_body_of_a_read_and_of_a_list_page_has_a_limit_and_a_watch_has_none() {
    let bed = Bed::start(None).await;
    // The path gives a body that stays open, as a server that stops in the middle of an answer.
    let read = bed.fetch(RequestKind::Get, "/watch/a").await.expect_err("a body with no end");
    assert_eq!((bed.elapsed(), read.to_string().as_str()), (10 * SECOND, "the answer was not complete in 10s"));

    let list = bed.fetch(RequestKind::ListPage, "/watch/b").await.expect_err("a body with no data");
    assert_eq!((bed.elapsed(), list.to_string().as_str()), (25 * SECOND, "the answer had no data for 15s"));

    let mut watch = bed.send(RequestKind::Watch, "/watch/c").await.expect("a stream").into_body();
    sleep(600 * SECOND).await;
    bed.server.send_event("late event\n");
    let event = watch.frame().await.expect("a frame").expect("no time limit for a watch");
    assert_eq!(event.into_data().expect("data"), "late event\n");
}

#[tokio::test(start_paused = true)]
async fn a_request_with_no_kind_is_refused_and_the_tower_service_sends_a_request() {
    let bed = Bed::start(None).await;
    let no_kind = Request::get("/api/v1/pods").body(Bytes::new()).expect("a request");
    let error = bed.cluster.send(no_kind).await.expect_err("no kind");
    assert_eq!(error.class(), ErrorClass::Invalid);
    assert!(bed.seen().is_empty(), "the request did not go to the server");

    let mut service = bed.cluster.clone();
    poll_fn(|cx| service.poll_ready(cx)).await.expect("always ready");
    let mut request = Request::get("https://other.test/api/v1/pods?limit=5").body(Bytes::new()).expect("a request");
    request.extensions_mut().insert(RequestKind::Get);
    assert_eq!(service.call(request).await.expect("an answer").status(), StatusCode::OK);
    assert_eq!(bed.seen()[0].path, "/api/v1/pods");
}

#[tokio::test(start_paused = true)]
async fn a_read_asks_for_gzip_and_gets_the_text_and_a_watch_does_not_ask() {
    let bed = Bed::start(None).await;
    let answer = bed.send(RequestKind::ListPage, "/big").await.expect("an answer");
    assert!(!answer.headers().contains_key(CONTENT_ENCODING), "the caller sees a plain body");
    let text = answer.into_body().collect().await.expect("a body").to_bytes();
    assert!(text == big_text().as_bytes(), "{} bytes", text.len());

    drop(bed.send(RequestKind::Watch, "/watch/pods").await.expect("a stream"));
    drop(bed.send(RequestKind::Write, "/api/v1/pods").await.expect("an answer"));
    let asked: Vec<bool> = bed.seen().iter().map(|seen| seen.headers.contains_key(ACCEPT_ENCODING)).collect();
    assert_eq!(asked, [true, false, false], "a list, a watch and a write");
    let lanes: Vec<u64> = bed.seen().iter().map(|seen| seen.connection).collect();
    assert_ne!(lanes[0], lanes[1], "a watch has its own connection");
    assert_eq!(lanes[0], lanes[2], "a write uses the connection of the requests");
}

#[tokio::test(start_paused = true)]
async fn the_app_obeys_retry_after_and_shows_the_throttle() {
    let bed = Bed::start(None).await;
    bed.server.throttle_next(&["2"]);
    let (first, second) = tokio::join!(bed.fetch(RequestKind::Get, "/api/v1/pods"), async {
        sleep(SECOND).await;
        let status = bed.cluster.status().throttle;
        assert_eq!((status.until, status.answers), (Some(epoch() + 2 * SECOND), 1), "throttled until");
        bed.fetch(RequestKind::Get, "/api/v1/nodes").await
    });
    assert_eq!(first.expect("an answer").0, StatusCode::OK, "the request went out again after the wait");
    assert_eq!(second.expect("an answer").0, StatusCode::OK);
    assert_eq!(bed.elapsed(), 2 * SECOND, "the second request waited for the end of the throttle");
    assert_eq!(bed.seen().len(), 3);
    assert_eq!(bed.cluster.status().throttle.until, None);

    // A wait that is longer than the time limit of the request: the request fails at once.
    bed.server.throttle_next(&["20"]);
    let error = bed.fetch(RequestKind::Get, "/api/v1/pods").await.expect_err("a long throttle");
    assert_eq!((error.class(), bed.elapsed()), (ErrorClass::Server, 2 * SECOND));
    assert_eq!(error.to_string(), "the server throttles requests for 20s more (HTTP 429)");
}

#[tokio::test(start_paused = true)]
async fn an_http_401_of_one_path_does_not_change_other_requests() {
    let bed = Bed::start(Some(&["one", "two"])).await;
    bed.server.refuse_path("/apis/metrics.k8s.io/");
    let (metrics, pods) = tokio::join!(
        bed.fetch(RequestKind::Get, METRICS),
        futures_util::future::join_all((0..20).map(|_| bed.fetch(RequestKind::ListPage, "/api/v1/pods")))
    );
    assert_eq!(metrics.expect("the answer of the server").0, StatusCode::UNAUTHORIZED);
    assert!(pods.iter().all(|answer| matches!(answer, Ok((StatusCode::OK, _)))), "{pods:?}");

    let credential = bed.cluster.status().credential.expect("a credential");
    assert_eq!(credential.state, CredentialState::Valid { until: None });
    assert_eq!((credential.plugin_runs, credential.unauthorized, credential.proofs), (2, 2, 1));
    // The proof is a light request with the new token on a request channel.
    let proof = bed.server.seen().into_iter().rfind(|seen| seen.path == "/version").expect("the proof");
    assert_eq!(proof.headers[AUTHORIZATION], "Bearer two");
    assert_eq!(bed.elapsed(), Duration::ZERO, "no wait in the path of a request");

    for _ in 0..3 {
        sleep(5 * SECOND).await;
        assert_eq!(bed.fetch(RequestKind::Get, METRICS).await.expect("an answer").0, StatusCode::UNAUTHORIZED);
        assert_eq!(bed.fetch(RequestKind::Get, "/api/v1/pods").await.expect("an answer").0, StatusCode::OK);
    }
    let credential = bed.cluster.status().credential.expect("a credential");
    assert_eq!(
        (credential.plugin_runs, credential.proofs),
        (2, 1),
        "the path costs no more plugin run and no more proof"
    );
}

#[tokio::test(start_paused = true)]
async fn the_light_request_of_a_channel_carries_the_token() {
    let bed = Bed::start(Some(&["one"])).await;
    sleep(31 * SECOND).await;
    let light: Vec<_> = bed.server.seen().into_iter().filter(|seen| seen.path == "/version").collect();
    assert_eq!(light.len(), 6, "three channels: the first request and one after 30 s");
    for request in &light[3..] {
        assert_eq!(request.headers[AUTHORIZATION], "Bearer one");
    }
    let status = bed.cluster.status();
    assert_eq!((status.name.as_str(), status.connect.connects, status.upgrade.warm), ("memory", 3, 0));
}

/// A defect in the credential task must show, and the task must run again (design rule 1).
#[tokio::test(start_paused = true)]
async fn a_panic_in_the_credential_task_gives_a_report_and_the_task_runs_again() {
    let mut bed = Bed::start(Some(&["panic", "one"])).await;
    let status = bed
        .until("the end of the task shows", |status| {
            !matches!(
                status.credential.as_ref().map(|credential| &credential.state),
                Some(CredentialState::Running { .. })
            )
        })
        .await;
    let state = status.credential.expect("a credential").state;
    assert!(
        matches!(state, CredentialState::Failed { failure: crate::credential::CredentialFailure::Stopped, .. }),
        "{state:?}"
    );

    bed.until("a valid credential", |status| {
        matches!(status.credential.as_ref().map(|credential| &credential.state), Some(CredentialState::Valid { .. }))
    })
    .await;
    assert_eq!(bed.fetch(RequestKind::Get, "/api/v1/pods").await.expect("an answer").0, StatusCode::OK);
    assert_eq!(bed.seen()[0].headers[AUTHORIZATION], "Bearer one");
    let reports = std::mem::take(&mut *bed.reports.lock().expect("a lock"));
    assert_eq!(reports.len(), 1, "{reports:?}");
    assert!(
        reports[0].starts_with("task credential [memory] panicked: assertion `left != right` failed: the token source"),
        "{reports:?}"
    );
}
