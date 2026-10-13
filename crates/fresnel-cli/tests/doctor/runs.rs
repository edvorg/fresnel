//! Runs of the program against the fake API server.

use std::net::SocketAddr;
use std::path::Path;
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use fresnel_testkit::{FakeApiServer, FaultProxy, ProxyMode};
use http::Method;

use crate::home::Home;

/// The command of the plugin in the kubeconfig: a bare name, which only the profile finds.
const PLUGIN: &str = "fresnel-test-plugin";

/// A home folder with the plugin of the tests of `fresnel-net` in its folder `plugins`.
fn home(label: &str) -> Home {
    let home = Home::new(label, &["kubeconfig.yaml"]);
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fresnel-net/tests/fixtures/fake-plugin");
    home.add_program(PLUGIN, &format!("#!/bin/sh\nexec '{}'\n", fixture.display()));
    home
}

/// How the user of a context of the test kubeconfig proves the identity.
enum User {
    /// No user entry.
    None,
    /// The plugin of the tests. Its token is `<token>-<number of the run>`.
    Plugin { token: &'static str },
    /// A static token, which the app does not support.
    Token,
}

/// The text of a kubeconfig with one context for each line. The address is the fake
/// server or a proxy before it.
fn kubeconfig(home: &Home, contexts: &[(&str, SocketAddr, &FakeApiServer, User)]) -> String {
    let (mut clusters, mut users, mut list) = (String::new(), String::new(), String::new());
    for (name, address, server, user) in contexts {
        let certificate = STANDARD.encode(server.certificate_der());
        let authority =
            STANDARD.encode(format!("-----BEGIN CERTIFICATE-----\n{certificate}\n-----END CERTIFICATE-----\n"));
        clusters.push_str(&format!(
            "- {{name: {name}, cluster: {{server: 'https://{address}', certificate-authority-data: {authority}}}}}\n"
        ));
        let state = home.path("state");
        let state = state.display();
        match user {
            User::None => list.push_str(&format!("- {{name: {name}, context: {{cluster: {name}}}}}\n")),
            User::Plugin { token } => users.push_str(&format!(
                "- name: {name}
  user:
    exec:
      apiVersion: client.authentication.k8s.io/v1beta1
      command: {PLUGIN}
      env: [{{name: FAKE_PLUGIN_STATE, value: '{state}'}}, {{name: FAKE_PLUGIN_TOKEN, value: {token}}}]
"
            )),
            User::Token => users.push_str(&format!("- {{name: {name}, user: {{token: static-token-of-the-test}}}}\n")),
        }
        if !matches!(user, User::None) {
            list.push_str(&format!("- {{name: {name}, context: {{cluster: {name}, user: {name}}}}}\n"));
        }
    }
    format!("clusters:\n{clusters}users:\n{users}contexts:\n{list}")
}

/// The requests of a run. The command must send no request but `GET /version`.
fn only_the_version(server: &FakeApiServer) -> usize {
    let requests = server.requests();
    for seen in &requests {
        assert_eq!((&seen.method, seen.uri.path(), seen.uri.query()), (&Method::GET, "/version", None), "{seen:?}");
    }
    requests.len()
}

/// The acceptance of milestone M0 for the command: it starts with the environment of
/// Finder, finds the kubeconfig and the plugin through the login shell, and measures
/// through a network with held connections.
#[tokio::test(flavor = "multi_thread")]
async fn the_command_measures_a_cluster_with_the_environment_of_finder() {
    let server = FakeApiServer::start().await.expect("a server");
    let proxy = FaultProxy::start(server.addr()).await.expect("a proxy");
    proxy.set(ProxyMode::HoldNew { probability: 0.3, min: Duration::from_secs(3), max: Duration::from_secs(20) });
    let home = home("finder");
    home.write(
        "work/kubeconfig.yaml",
        &kubeconfig(&home, &[("fake", proxy.addr(), &server, User::Plugin { token: "doctor" })]),
    );

    let run = home.doctor(&["--context", "fake", "--samples", "8", "--connect-limit", "1"]).await;
    assert_eq!(run.status, Some(0), "stdout:\n{}\nstderr:\n{}", run.stdout, run.stderr);
    println!("{}", run.stdout);
    let report = home.json("fresnel-doctor.json");

    // The login shell gave `KUBECONFIG` and the folder of the plugin.
    let environment = &report["environment"];
    assert!(environment["source"].as_str().is_some_and(|source| source.starts_with("login shell /")), "{environment}");
    assert_eq!((&environment["problem"], &environment["process_path_entries"]), (&serde_json::Value::Null, &4.into()));
    let kubeconfig_file = home.path("work/kubeconfig.yaml").display().to_string();
    assert_eq!(report["kubeconfig"]["sources"][0], kubeconfig_file);
    assert_eq!(report["kubeconfig"]["absent"][0], home.path(".kube/config").display().to_string());
    let cluster = &report["clusters"][0];
    assert_eq!((&cluster["context"], &cluster["server"]), (&"fake".into(), &proxy.addr().to_string().into()));
    assert_eq!(cluster["auth"], format!("plugin {PLUGIN}"));
    let plugin = &cluster["plugin"];
    assert_eq!(plugin["folder"], home.path("plugins").display().to_string());
    assert_eq!(plugin["in_process_path"], false, "only the login shell has the folder of the plugin");

    // The connects: some with no hedge are held, and none with the hedge.
    let result = &cluster["result"];
    assert_eq!(
        (&result["dns"]["time_ms"], &result["dns"]["addresses"][0]),
        (&0.0.into(), &proxy.addr().to_string().into())
    );
    assert_eq!((&result["alpn"]["http2"], &result["alpn"]["http1"]), (&true.into(), &true.into()));
    let (plain, hedged) = (&result["plain"], &result["hedged"]);
    let held = plain["held"].as_u64().expect("a count");
    let no_time = plain["times_ms"].as_array().expect("times").iter().filter(|time| time.is_null()).count() as u64;
    assert!(
        held >= 1 && held == no_time,
        "the proxy held connections, and each one is a sample with no answer: {plain}"
    );
    assert_eq!(plain["connected"].as_u64().expect("a count") + held, 8, "{plain}");
    assert_eq!(plain["errors"][0], "no answer in 1s");
    assert_eq!(
        (&hedged["samples"], &hedged["connected"], &hedged["held"]),
        (&8.into(), &8.into(), &0.into()),
        "{hedged}"
    );
    assert!(hedged["attempts"].as_u64().expect("a count") > 8, "the hedge started a second attempt: {hedged}");
    assert!(hedged["total_ms"]["max"].as_f64().expect("a time") < 1000.0, "{hedged}");

    // The session: one plugin run, the limit of the server and its version.
    let session = &result["session"];
    let credential = &session["credential"];
    assert_eq!((&credential["plugin_runs"], &credential["state"]), (&1.into(), &"the credential is valid".into()));
    assert!(credential["first_token_ms"].as_f64().expect("a time") > 0.0);
    assert_eq!(home.read("state/runs"), "run\n", "the plugin process ran one time");
    assert_eq!((&session["stream_limit"], &session["idle_close"]), (&100.into(), &serde_json::Value::Null));
    assert_eq!(session["version"]["git_version"], "v1.33.5+fake");
    assert!(session["ping_ms"]["max"].as_f64().expect("a time") < 1000.0, "{session}");
    assert!(session["live_ms"].as_f64().is_some(), "{session}");
    assert_eq!(report["task_reports"], serde_json::json!([]));

    // The command read no object, and the token is in no output.
    assert!(only_the_version(&server) >= 5, "five small requests and the light requests of the channels");
    assert!(
        server.requests().iter().any(|seen| seen.bearer() == Some("doctor-1")),
        "the token of the plugin was in use"
    );
    assert!(run.stdout.contains("GET /version  v1.33.5+fake (linux/arm64)"), "{}", run.stdout);
    for output in [&run.stdout, &run.stderr, &home.read("fresnel-doctor.json")] {
        assert!(!output.contains("doctor-1"), "the token is in an output:\n{output}");
    }
    assert_eq!((proxy.task_failures(), server.task_failures()), (Vec::<String>::new(), Vec::new()));
}

/// A kube-apiserver ends a connection after a request with no credential. A channel of a
/// cluster with a plugin can send its light request before the first token. Such a channel
/// is on a later connection when it is live, and that is not the close of an idle connection.
#[tokio::test(flavor = "multi_thread")]
async fn a_connection_that_the_server_ended_before_the_first_token_is_no_idle_close() {
    let server = FakeApiServer::start().await.expect("a server");
    server.end_anonymous_connections();
    let home = home("anonymous");
    // The plugin is slow, so each channel sends a request with no token.
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fresnel-net/tests/fixtures/fake-plugin");
    home.add_program(PLUGIN, &format!("#!/bin/sh\nsleep 0.3\nexec '{}'\n", fixture.display()));
    home.write(
        "work/kubeconfig.yaml",
        &kubeconfig(&home, &[("fake", server.addr(), &server, User::Plugin { token: "doctor" })]),
    );

    let run = home.doctor(&["--context", "fake", "--samples", "0", "--idle-close", "2"]).await;
    assert_eq!(run.status, Some(0), "stdout:\n{}\nstderr:\n{}", run.stdout, run.stderr);
    let session = &home.json("fresnel-doctor.json")["clusters"][0]["result"]["session"];
    let channels = session["channels"].as_array().expect("the channels");
    let connections: u64 = channels.iter().filter_map(|channel| channel["connections"].as_u64()).sum();
    assert!(connections > 3, "the fault was not present: no channel lost a connection: {channels:?}");
    let idle = &session["idle_close"];
    assert_eq!(
        (&idle["closed_after_s"], &idle["light_request_kept_open"]),
        (&serde_json::Value::Null, &true.into()),
        "{idle}"
    );
    assert!(idle["waited_s"].as_f64().is_some_and(|waited| (2.0..3.0).contains(&waited)), "{idle}");
    assert_eq!(session["credential"]["plugin_runs"], 1);
    only_the_version(&server);
}

/// A server that is gone. No step can work with no connection, so the command does not
/// wait for its samples and for its channels.
#[tokio::test(flavor = "multi_thread")]
async fn a_server_that_refuses_each_connection_gives_its_report_with_no_wait() {
    let server = FakeApiServer::start().await.expect("a server");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a free port");
    let gone = listener.local_addr().expect("its address");
    drop(listener);
    let home = home("refused");
    home.write("work/kubeconfig.yaml", &kubeconfig(&home, &[("gone", gone, &server, User::None)]));

    let started = Instant::now();
    let run = home.doctor(&["--context", "gone"]).await;
    let elapsed = started.elapsed();
    assert_eq!(run.status, Some(1), "stdout:\n{}\nstderr:\n{}", run.stdout, run.stderr);
    assert!(elapsed < Duration::from_secs(10), "the run took {elapsed:?}:\n{}", run.stdout);
    let result = &home.json("fresnel-doctor.json")["clusters"][0]["result"];
    for protocol in ["http2", "http1"] {
        let error = result["alpn"][protocol]["error"].as_str().expect("the error of the connect");
        assert!(error.contains("refused"), "{error}");
    }
    assert_eq!((&result["plain"], &result["hedged"]), (&serde_json::Value::Null, &serde_json::Value::Null));
    assert_eq!(result["session"]["error"], "not connected: no HTTP/2 connection to the server");
    assert_eq!(server.counters().connections, 0);
}

/// The acceptance of milestone M0: a plugin that hangs shows a timeout. The command ends
/// after the time limit of the plugin, and no process of the plugin stays.
#[tokio::test(flavor = "multi_thread")]
async fn a_plugin_that_hangs_shows_a_timeout_and_leaves_no_process() {
    let server = FakeApiServer::start().await.expect("a server");
    server.end_anonymous_connections();
    let home = home("hang");
    home.write(
        "work/kubeconfig.yaml",
        &kubeconfig(&home, &[("fake", server.addr(), &server, User::Plugin { token: "doctor" })]),
    );
    home.write("state/mode", "hang\n");

    let started = Instant::now();
    let run = home.doctor(&["--context", "fake", "--samples", "0", "--idle-close", "30"]).await;
    let elapsed = started.elapsed();
    assert_eq!(run.status, Some(1), "stdout:\n{}\nstderr:\n{}", run.stdout, run.stderr);
    assert!((Duration::from_secs(20)..Duration::from_secs(25)).contains(&elapsed), "the run took {elapsed:?}");
    let session = &home.json("fresnel-doctor.json")["clusters"][0]["result"]["session"];
    let failure = format!("credential plugin `{PLUGIN}`: no answer in 20s");
    assert_eq!((&session["credential"]["state"], &session["credential"]["plugin_runs"]), (&failure.into(), &1.into()));
    assert_eq!(session["version"]["error"], session["credential"]["state"]);
    assert_eq!(session["idle_close"], serde_json::Value::Null, "no idle wait with no live channel");
    assert!(run.stdout.contains("plugin: fake-plugin: waiting for a login"), "{}", run.stdout);
    for file in ["state/pid", "state/child"] {
        let pid = home.read(file);
        let alive = std::process::Command::new("/bin/kill").args(["-0", pid.trim()]).output().expect("kill runs");
        assert!(!alive.status.success(), "the process {} of the plugin is alive ({file})", pid.trim());
    }
}

/// Headlamp weakness 1: data from the first selected cluster only.
#[tokio::test(flavor = "multi_thread")]
async fn the_command_measures_each_selected_cluster_on_its_own_server() {
    let (east, west) =
        (FakeApiServer::start().await.expect("a server"), FakeApiServer::start().await.expect("a server"));
    let home = home("two");
    let contexts = [
        ("east", east.addr(), &east, User::None),
        ("west", west.addr(), &west, User::None),
        ("token", west.addr(), &west, User::Token),
    ];
    home.write("config.yaml", &kubeconfig(&home, &contexts));
    let file = home.path("config.yaml").display().to_string();

    // With no selected context, the command lists the contexts and opens no connection.
    let run = home.doctor(&["--kubeconfig", &file]).await;
    assert_eq!(run.status, Some(0), "{}", run.stderr);
    let expected = format!(
        "3 contexts in 1 kubeconfig file:\n  east  {}  no credential\n  west  {}  no credential\n  \
         token  not usable: the user entry `token` has a static token; the app does not support that yet\n\
         Select a context with --context <name>, or each context with --all.\n",
        east.addr(),
        west.addr()
    );
    assert_eq!(run.stdout, expected);
    assert_eq!((east.counters().connections, west.counters().connections), (0, 0));
    let run = home.doctor(&["--kubeconfig", &file, "--context", "north"]).await;
    assert_eq!(run.status, Some(2));
    assert!(run.stderr.starts_with("the kubeconfig files have no context `north`\n\nusage: "), "{}", run.stderr);
    assert_eq!((east.counters().connections, west.counters().connections), (0, 0));

    let json = home.path("out.json").display().to_string();
    let run =
        home.doctor(&["--kubeconfig", &file, "--all", "--samples", "2", "--idle-close", "2", "--json", &json]).await;
    assert_eq!(run.status, Some(1), "the context `token` gives no version:\n{}\n{}", run.stdout, run.stderr);
    let report = home.json("out.json");
    assert_eq!(report["kubeconfig"]["sources"], serde_json::json!([file]), "the option replaces the files of the user");
    let clusters = report["clusters"].as_array().expect("the clusters");
    assert_eq!(clusters.len(), 3);
    for (cluster, server) in clusters.iter().zip([&east, &west]) {
        assert_eq!(
            (&cluster["server"], &cluster["auth"]),
            (&server.addr().to_string().into(), &"no credential".into())
        );
        let session = &cluster["result"]["session"];
        assert_eq!(
            (&session["version"]["git_version"], &session["credential"]),
            (&"v1.33.5+fake".into(), &serde_json::Value::Null)
        );
        // The fake server never closes an idle connection.
        let idle = &session["idle_close"];
        assert_eq!(
            (&idle["closed_after_s"], &idle["light_request_kept_open"]),
            (&serde_json::Value::Null, &true.into())
        );
        assert!(idle["waited_s"].as_f64().is_some_and(|waited| (2.0..3.0).contains(&waited)), "{idle}");
        // Two connects for the protocols, two with no hedge, two with it, and three channels.
        assert_eq!(server.counters().connections, 9);
        assert!(only_the_version(server) >= 5);
    }
    let unusable = &clusters[2];
    assert_eq!((&unusable["context"], &unusable["server"]), (&"token".into(), &serde_json::Value::Null));
    let reason = "the user entry `token` has a static token; the app does not support that yet";
    assert_eq!(unusable["result"]["error"], reason);
    assert!(run.stdout.contains("the idle connection was open after 2."), "{}", run.stdout);
    assert!(!home.read("out.json").contains("static-token-of-the-test"));
}

/// The text of a plugin that fails shows at the terminal, and the JSON file has the
/// failure type only.
#[tokio::test(flavor = "multi_thread")]
async fn a_plugin_that_fails_shows_its_text_at_the_terminal_and_not_in_the_file() {
    let server = FakeApiServer::start().await.expect("a server");
    server.end_anonymous_connections();
    let home = home("fail");
    home.write(
        "work/kubeconfig.yaml",
        &kubeconfig(&home, &[("fake", server.addr(), &server, User::Plugin { token: "doctor" })]),
    );
    home.write("state/mode", "fail\n");

    let run = home.doctor(&["--all", "--samples", "0"]).await;
    assert_eq!(run.status, Some(1), "stdout:\n{}\nstderr:\n{}", run.stdout, run.stderr);
    let session = &home.json("fresnel-doctor.json")["clusters"][0]["result"]["session"];
    let failure = format!("credential plugin `{PLUGIN}`: stopped with exit status: 7");
    assert_eq!(session["credential"]["state"], failure);
    assert_eq!(session["credential"]["first_token_ms"], serde_json::Value::Null);
    assert_eq!(session["version"]["error"], failure, "the request fails with the state of the credential");

    let text_of_the_plugin = "Error: access token is required (secret-text-of-the-plugin)";
    assert!(run.stdout.contains(&format!("plugin: {text_of_the_plugin}")), "{}", run.stdout);
    assert!(!home.read("fresnel-doctor.json").contains("secret-text"), "the file has the text of the plugin");
    assert!(server.requests().iter().all(|seen| seen.bearer().is_none()), "no request had a token");
    only_the_version(&server);
}
