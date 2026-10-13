//! The `doctor` command on a real kube-apiserver: k3s in Docker behind the fault proxy.
//!
//! The program runs as a child process with the environment of a program from Finder.

#![allow(clippy::expect_used, reason = "a test stops with a panic")]

#[path = "doctor/home.rs"]
mod home;

use std::time::Duration;

use fresnel_testkit::{FaultProxy, K3s, ProxyMode};
use serde_json::Value;

use home::Home;

/// A plugin with a cache: it gives the token of a file.
const TOKEN_PLUGIN: &str = "#!/bin/sh
printf '{\"apiVersion\":\"client.authentication.k8s.io/v1\",\"kind\":\"ExecCredential\",\"status\":{\"token\":\"%s\"}}\\n' \\
    \"$(cat \"$HOME/work/token\")\"
";
/// A second kubeconfig file. Its context uses the cluster entry of the file of k3s.
const TOKEN_CONTEXT: &str = "\
users:
- name: reader
  user: {exec: {apiVersion: client.authentication.k8s.io/v1, command: fresnel-test-token, interactiveMode: Never}}
contexts:
- {name: with-plugin, context: {cluster: default, user: reader}}
";

fn number(value: &Value, what: &str) -> f64 {
    value.as_f64().unwrap_or_else(|| panic!("{what} is a number: {value}"))
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs k3s in Docker"]
async fn the_command_measures_a_real_kube_apiserver_through_the_fault_proxy() {
    let k3s = K3s::start("doctor").await.expect("k3s starts");
    let proxy = FaultProxy::start(k3s.api_addr()).await.expect("a proxy");
    // A far cluster: each byte needs 20 ms in each direction.
    proxy.set(ProxyMode::Latency(Duration::from_millis(20)));
    let home = Home::new("k3s", &["kubeconfig.yaml", "token.yaml"]);
    let admin = std::fs::read_to_string(k3s.kubeconfig_through(proxy.addr()).expect("a kubeconfig")).expect("its text");
    home.write("work/kubeconfig.yaml", &admin);
    home.write("work/token.yaml", TOKEN_CONTEXT);
    home.add_program("fresnel-test-token", TOKEN_PLUGIN);
    // A new cluster has no service account for some seconds, so the test makes its own.
    k3s.kubectl(&["create", "serviceaccount", "reader", "--namespace", "default"]).await.expect("a service account");
    let token = k3s.kubectl(&["create", "token", "reader", "--namespace", "default"]).await.expect("a token");
    home.write("work/token", token.trim());

    // The context of the cluster admin, with a client certificate.
    let run = home.doctor(&["--context", "default", "--samples", "5"]).await;
    println!("{}", run.stdout);
    assert_eq!(run.status, Some(0), "stderr:\n{}", run.stderr);
    let report = home.json("fresnel-doctor.json");
    assert_eq!(report["kubeconfig"]["contexts"], 2, "the two files of KUBECONFIG are one list");
    let cluster = &report["clusters"][0];
    assert_eq!((&cluster["auth"], &cluster["plugin"]), (&"client certificate".into(), &Value::Null));
    let result = &cluster["result"];
    assert_eq!((&result["alpn"]["http2"], &result["alpn"]["http1"]), (&true.into(), &true.into()));
    let (plain, hedged) = (&result["plain"], &result["hedged"]);
    assert_eq!(
        (&plain["connected"], &plain["held"], &hedged["connected"], &hedged["held"]),
        (&5.into(), &0.into(), &5.into(), &0.into())
    );
    // The proxy holds a new connection for one round trip, and the TLS handshake needs one more.
    assert!(number(&plain["tls_ms"]["min"], "the TLS time") >= 80.0, "{plain}");
    assert!(number(&hedged["total_ms"]["min"], "the connect time") >= 80.0, "{hedged}");
    assert_eq!(hedged["attempts"], 5, "a far server does not get two attempts for each connect: {hedged}");
    let session = &result["session"];
    assert_eq!((&session["stream_limit"], &session["credential"]), (&100.into(), &Value::Null));
    assert_eq!(session["version"]["git_version"], "v1.33.5+k3s1");
    assert!(number(&session["ping_ms"]["min"], "the round trip") >= 40.0, "{session}");
    assert_eq!((&session["ping_error"], &session["idle_close"]), (&Value::Null, &Value::Null));
    let connections: Vec<_> =
        session["channels"].as_array().expect("channels").iter().map(|c| &c["connections"]).collect();
    assert_eq!(connections, [1, 1, 1], "a client certificate needs one connection for each channel");

    // The context with a credential plugin, with the wait for the close of an idle
    // connection. The login shell gives the folder of the plugin and the kubeconfig.
    let json = home.path("plugin.json").display().to_string();
    let options = ["--context", "with-plugin", "--samples", "0", "--idle-close", "120", "--json", &json];
    let run = home.doctor(&options).await;
    println!("{}", run.stdout);
    assert_eq!(run.status, Some(0), "stderr:\n{}", run.stderr);
    let report = home.json("plugin.json");
    let cluster = &report["clusters"][0];
    assert_eq!(cluster["auth"], "plugin fresnel-test-token");
    assert_eq!(cluster["plugin"]["folder"], home.path("plugins").display().to_string());
    assert_eq!(cluster["plugin"]["in_process_path"], false);
    let session = &cluster["result"]["session"];
    println!("the channels of the run with a plugin: {:#}", session["channels"]);
    assert_eq!(
        (&session["credential"]["plugin_runs"], &session["credential"]["state"]),
        (&1.into(), &"the credential is valid".into())
    );
    // The server refuses a request with a bad token, also for this path.
    assert_eq!(session["version"]["git_version"], "v1.33.5+k3s1");
    // A kube-apiserver closes a connection with no request after 90 s. The light request
    // of the other channels prevents that. A connection that the server ended before the
    // first token is not such a close.
    let idle = &session["idle_close"];
    let closed_after = number(&idle["closed_after_s"], "the time of the close");
    assert!((85.0..100.0).contains(&closed_after), "{idle}");
    assert_eq!(
        (&idle["reason"], &idle["light_request_kept_open"]),
        (&"the server closed the connection".into(), &true.into())
    );
    for output in [&run.stdout, &run.stderr, &home.read("plugin.json")] {
        assert!(!output.contains(token.trim()), "the token is in an output");
    }
    assert_eq!(proxy.task_failures(), Vec::<String>::new());
}
