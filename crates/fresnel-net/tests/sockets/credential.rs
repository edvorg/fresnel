//! The credential of a cluster with a real plugin process: the acceptance lines of
//! milestone M0 for the plugin, and the process facts that a scripted source cannot show.

use std::time::{Duration, Instant};

use fresnel_net::credential::{CredentialFailure, CredentialState, PluginFailure};
use fresnel_net::{Cause, Cluster, ClusterSpec, ErrorClass, RequestKind};
use fresnel_testkit::{FakeApiServer, Fault};
use http::StatusCode;

use crate::common::{
    Scratch, TestTasks, fetch, finder_env, plugin, process_exists, send, spec, trust, until, until_live,
};

const SECOND: Duration = Duration::from_secs(1);
const METRICS: &str = "/apis/metrics.k8s.io/v1beta1/nodes";

fn with_plugin(name: &str, server: &FakeApiServer, state: &Scratch) -> ClusterSpec {
    ClusterSpec { plugin: Some(plugin(state, name)), env: finder_env(), ..spec(name, server.addr(), trust(server)) }
}

async fn until_valid(cluster: &Cluster) {
    until(cluster, "a valid credential", |status| {
        matches!(status.credential.as_ref().map(|credential| &credential.state), Some(CredentialState::Valid { .. }))
    })
    .await;
}

/// The count of plugin runs, of HTTP 401 answers and of proofs.
fn counts(cluster: &Cluster) -> (u64, u64, u64) {
    let credential = cluster.status().credential.expect("a credential");
    (credential.plugin_runs, credential.unauthorized, credential.proofs)
}

/// "The plugin runs one time for each cluster."
#[tokio::test]
async fn the_plugin_runs_one_time_for_each_cluster() {
    let tasks = TestTasks::new();
    let (first_server, second_server) =
        (FakeApiServer::start().await.expect("a server"), FakeApiServer::start().await.expect("a server"));
    let (first_state, second_state) = (Scratch::new("plugin-a"), Scratch::new("plugin-b"));
    let first = Cluster::start(&tasks.tasks, with_plugin("a", &first_server, &first_state)).expect("a cluster");
    let second = Cluster::start(&tasks.tasks, with_plugin("b", &second_server, &second_state)).expect("a cluster");

    // Requests of each kind start at once, before the first token exists.
    for cluster in [&first, &second] {
        let reads = (0..10).map(|_| fetch(cluster, RequestKind::ListPage, "/api/v1/pods"));
        let watches = (0..10).map(|_| send(cluster, RequestKind::Watch, "/api/v1/pods?watch=true"));
        let (reads, watches) =
            tokio::join!(futures_util::future::join_all(reads), futures_util::future::join_all(watches));
        assert!(reads.iter().all(|answer| matches!(answer, Ok((StatusCode::OK, _)))), "{reads:?}");
        assert!(watches.iter().all(|answer| answer.as_ref().is_ok_and(|answer| answer.status() == StatusCode::OK)));
        until_live(cluster).await;
    }

    for (cluster, state, server, token) in
        [(&first, &first_state, &first_server, "a-1"), (&second, &second_state, &second_server, "b-1")]
    {
        assert_eq!(counts(cluster), (1, 0, 0), "one plugin run and no refused request");
        assert_eq!(state.read("runs"), "run\n", "the plugin process ran one time");
        assert!(state.read("exec-info").contains(r#""interactive":false"#), "{}", state.read("exec-info"));
        // Each request of a caller has the token of its own cluster.
        let tokens: Vec<_> = server.requests().into_iter().filter(|request| request.uri.path() != "/version").collect();
        assert_eq!(tokens.len(), 20);
        assert!(
            tokens.iter().all(|request| request.bearer() == Some(token)),
            "a request with no token or an other one"
        );
    }
}

/// "A plugin that hangs shows a timeout."
#[tokio::test]
async fn a_plugin_that_hangs_shows_a_timeout_and_its_processes_are_gone() {
    let tasks = TestTasks::new();
    let server = FakeApiServer::start().await.expect("a server");
    let state = Scratch::new("plugin-hang");
    state.write("mode", "hang");
    let mut spec = with_plugin("hang", &server, &state);
    spec.settings.credential.plugin_limit = 3 * SECOND;
    let started = Instant::now();
    let cluster = Cluster::start(&tasks.tasks, spec).expect("a cluster");

    // A request does not wait for the full run of the plugin.
    let error = fetch(&cluster, RequestKind::Get, "/api/v1/pods").await.expect_err("no token");
    assert_eq!(
        (error.class(), error.to_string().as_str()),
        (ErrorClass::Auth, "no credential in 2s: the credential plugin runs")
    );
    assert!(started.elapsed() < 3 * SECOND, "the request waited {:?}", started.elapsed());
    let (script, child) = (state.read("pid"), state.read("child"));
    assert!(process_exists(&script) && process_exists(&child), "the plugin and its child process run");

    let status = until(&cluster, "a failed credential", |status| {
        matches!(status.credential.as_ref().map(|credential| &credential.state), Some(CredentialState::Failed { .. }))
    })
    .await;
    let shown_after = started.elapsed();
    assert!(3 * SECOND <= shown_after && shown_after < 5 * SECOND, "the timeout showed after {shown_after:?}");
    let Some(CredentialState::Failed { failure: CredentialFailure::Plugin(error), failures: 1, .. }) =
        status.credential.map(|credential| credential.state)
    else {
        panic!("the state after the time limit: {:?}", cluster.status().credential);
    };
    assert_eq!(error.failure(), &PluginFailure::Timeout { limit: 3 * SECOND, stopped: true });
    // The text of the plugin is for the UI. No text that can go to a log has it.
    assert!(error.detail().contains("waiting for a login that never comes"), "{}", error.detail());
    for text in [error.to_string(), format!("{error:?}"), format!("{:?}", cluster.status())] {
        assert!(!text.contains("secret-text-of-the-plugin"), "{text}");
    }
    assert!(!process_exists(&script), "the plugin process is gone");
    let gone = async {
        while process_exists(&child) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    crate::common::in_time("the child of the plugin is gone", gone).await;
    assert!(
        server.requests().iter().all(|request| request.uri.path() == "/version"),
        "no request went out with no token"
    );

    // The user repairs the plugin. "Retry now" starts the run at once.
    state.write("mode", "ok");
    cluster.signal().raise(Cause::RetryNow);
    until_valid(&cluster).await;
    assert_eq!(fetch(&cluster, RequestKind::Get, "/api/v1/pods").await.expect("an answer").0, StatusCode::OK);
}

/// "An HTTP 401 of one path does not change other requests."
#[tokio::test]
async fn an_http_401_of_one_path_does_not_change_other_requests() {
    let tasks = TestTasks::new();
    let server = FakeApiServer::start().await.expect("a server");
    let state = Scratch::new("plugin-401-path");
    let cluster = Cluster::start(&tasks.tasks, with_plugin("path", &server, &state)).expect("a cluster");
    until_valid(&cluster).await;
    // An aggregated API with a broken trust setup: its path refuses each token.
    server.add_fault(Fault::unauthorized().on_path("/apis/metrics.k8s.io/"));

    let other_requests = async {
        let mut answers = Vec::new();
        for _ in 0..40 {
            answers.push(fetch(&cluster, RequestKind::ListPage, "/api/v1/pods").await.map(|answer| answer.0));
        }
        answers
    };
    let (metrics, pods) = tokio::join!(fetch(&cluster, RequestKind::Get, METRICS), other_requests);
    let (status, body) = metrics.expect("the answer of the server, not an error of the credential");
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(body.contains(r#""reason":"Unauthorized""#), "the caller reads the status of the server: {body}");
    assert_eq!(pods, vec![Ok(StatusCode::OK); 40], "the other requests of the cluster");
    assert_eq!(counts(&cluster), (2, 2, 1), "one more plugin run, then one proof");
    assert!(matches!(cluster.status().credential.expect("a credential").state, CredentialState::Valid { .. }));

    // The sampler reads the path again. The token is proven, so nothing more runs.
    for _ in 0..5 {
        assert_eq!(fetch(&cluster, RequestKind::Get, METRICS).await.expect("an answer").0, StatusCode::UNAUTHORIZED);
        assert_eq!(fetch(&cluster, RequestKind::Get, "/api/v1/pods").await.expect("an answer").0, StatusCode::OK);
    }
    assert_eq!(counts(&cluster), (2, 7, 1));
    assert_eq!(state.read("runs"), "run\nrun\n");
    let proof = server.requests().into_iter().rfind(|request| request.uri.path() == "/version").expect("the proof");
    assert_eq!(proof.bearer(), Some("path-2"), "the proof used the new token on a core path");
}

/// The rule before the proof: one HTTP 401, one plugin run, and the request goes out again.
#[tokio::test]
async fn a_refused_token_costs_one_plugin_run_and_no_failed_request() {
    let tasks = TestTasks::new();
    let server = FakeApiServer::start().await.expect("a server");
    server.add_fault(Fault::unauthorized().for_token("first-1"));
    let state = Scratch::new("plugin-401-token");
    let cluster = Cluster::start(&tasks.tasks, with_plugin("first", &server, &state)).expect("a cluster");

    let answers =
        futures_util::future::join_all((0..20).map(|_| fetch(&cluster, RequestKind::Get, "/api/v1/pods"))).await;
    assert!(answers.iter().all(|answer| matches!(answer, Ok((StatusCode::OK, _)))), "{answers:?}");
    let (runs, unauthorized, proofs) = counts(&cluster);
    assert_eq!((runs, proofs), (2, 0), "20 refused requests cost one plugin run");
    assert!((1..=20).contains(&unauthorized), "{unauthorized}");
    assert_eq!(state.read("runs"), "run\nrun\n");
}

#[tokio::test]
async fn a_plugin_that_fails_shows_its_text_in_the_state_and_runs_again_by_itself() {
    let tasks = TestTasks::new();
    let server = FakeApiServer::start().await.expect("a server");
    let state = Scratch::new("plugin-fail");
    state.write("mode", "fail");
    let cluster = Cluster::start(&tasks.tasks, with_plugin("fail", &server, &state)).expect("a cluster");

    let error = fetch(&cluster, RequestKind::Get, "/api/v1/pods").await.expect_err("no token");
    let command = plugin(&state, "x").command;
    assert_eq!(error.to_string(), format!("credential plugin `{command}`: stopped with exit status: 7"));
    let status = cluster.status().credential.expect("a credential");
    let CredentialState::Failed { failure, .. } = &status.state else { panic!("{status:?}") };
    assert_eq!(failure.detail(), "Error: access token is required (secret-text-of-the-plugin)");
    assert!(!format!("{status:?}").contains("secret-text"), "{status:?}");

    // The user repairs the plugin. The next run starts by itself after the pause of 2 s.
    state.write("mode", "ok");
    let waited = Instant::now();
    until_valid(&cluster).await;
    assert!(waited.elapsed() < 4 * SECOND, "{:?}", waited.elapsed());
    assert_eq!(fetch(&cluster, RequestKind::Get, "/api/v1/pods").await.expect("an answer").0, StatusCode::OK);
}

/// A plugin can leave a helper process that holds its output pipes open. The run ends when
/// the plugin process ends, and not at the time limit.
#[tokio::test]
async fn a_plugin_that_leaves_a_helper_process_gives_its_token_at_once() {
    let tasks = TestTasks::new();
    let server = FakeApiServer::start().await.expect("a server");
    let state = Scratch::new("plugin-helper");
    state.write("mode", "helper");
    let started = Instant::now();
    let cluster = Cluster::start(&tasks.tasks, with_plugin("helper", &server, &state)).expect("a cluster");
    until_valid(&cluster).await;
    assert!(started.elapsed() < 3 * SECOND, "the run waited for the helper: {:?}", started.elapsed());
    assert_eq!(fetch(&cluster, RequestKind::Get, "/api/v1/pods").await.expect("an answer").0, StatusCode::OK);
    // The test removes the helper. The app does not: after the exit of the plugin, its
    // process group id can belong to a new process.
    let helper: i32 = state.read("child").trim().parse().expect("a process id");
    nix::sys::signal::kill(nix::unistd::Pid::from_raw(helper), nix::sys::signal::Signal::SIGKILL)
        .expect("the helper runs");
}

#[tokio::test]
async fn a_plugin_that_writes_too_much_shows_that_failure_and_not_a_timeout() {
    let tasks = TestTasks::new();
    let server = FakeApiServer::start().await.expect("a server");
    let state = Scratch::new("plugin-flood");
    state.write("mode", "flood");
    let started = Instant::now();
    let cluster = Cluster::start(&tasks.tasks, with_plugin("flood", &server, &state)).expect("a cluster");
    let status = until(&cluster, "a failed credential", |status| {
        matches!(status.credential.as_ref().map(|credential| &credential.state), Some(CredentialState::Failed { .. }))
    })
    .await;
    assert!(started.elapsed() < 5 * SECOND, "{:?}", started.elapsed());
    let Some(CredentialState::Failed { failure: CredentialFailure::Plugin(error), .. }) =
        status.credential.map(|credential| credential.state)
    else {
        panic!("the state: {:?}", cluster.status().credential);
    };
    assert_eq!(error.failure(), &PluginFailure::TooMuchOutput(1024 * 1024));
    assert!(error.detail().len() <= 2000 * 4, "the detail is the end of the text only");
}
