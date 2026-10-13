//! Runs of the `watch` command against the fake API server.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use fresnel_testkit::{FakeApiServer, Fault};

use crate::home::Home;

/// A home folder whose kubeconfig has the one context `fake` for the server.
fn home(label: &str, server: &FakeApiServer) -> Home {
    let home = Home::new(label, &["kubeconfig.yaml"]);
    let certificate = STANDARD.encode(server.certificate_der());
    let authority = STANDARD.encode(format!("-----BEGIN CERTIFICATE-----\n{certificate}\n-----END CERTIFICATE-----\n"));
    let cluster = format!("{{server: 'https://{}', certificate-authority-data: {authority}}}", server.addr());
    let text = format!(
        "clusters:\n- {{name: fake, cluster: {cluster}}}\ncontexts:\n- {{name: fake, context: {{cluster: fake}}}}\n"
    );
    home.write("work/kubeconfig.yaml", &text);
    home
}

/// The lines of the last report of a run.
fn last_report(stdout: &str) -> Vec<&str> {
    let start = stdout.rfind("fresnel-cli watch, ").expect("a report");
    stdout[start..].lines().collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_command_shows_the_state_and_the_object_count_of_each_store() {
    let server = FakeApiServer::start().await.expect("a server");
    let pod = |name: &str| format!(r#"{{"metadata":{{"name":"{name}","namespace":"shop","resourceVersion":"5"}}}}"#);
    let pods = format!(r#"{{"metadata":{{"resourceVersion":"7"}},"items":[{},{}]}}"#, pod("api"), pod("web"));
    server.set_body("/api/v1/pods", &pods);
    let home = home("watch", &server);

    // The login shell gives the kubeconfig file. Three reports in three seconds.
    let run = home.run("watch", &["--context", "fake", "--interval", "1", "--for", "3"]).await;
    assert_eq!(run.status, Some(0), "stdout:\n{}\nstderr:\n{}", run.stdout, run.stderr);
    assert_eq!(run.stdout.matches("fresnel-cli watch, ").count(), 3, "{}", run.stdout);
    let report = last_report(&run.stdout);
    assert_eq!(report[2], "cluster fake: 39 of 39 stores live, 2 objects, 3 connections, 39 watch streams");
    assert_eq!(report.len(), 3 + 39, "one line for each store: {report:#?}");
    assert!(report[5].starts_with("  Pod                               A        2  live, last data "), "{}", report[5]);
    assert!(report[3 + 13].starts_with("  Secret                            B        0  live, "), "{}", report[16]);
    assert_eq!(run.stderr, "", "no task of the app ended");

    // A store that is not live at the end gives the exit status 1, with its reason.
    server.add_fault(Fault::forbidden().on_path("/api/v1/secrets"));
    let run = home.run("watch", &["--all", "--interval", "2", "--for", "2"]).await;
    assert_eq!(run.status, Some(1), "stdout:\n{}\nstderr:\n{}", run.stdout, run.stderr);
    let report = last_report(&run.stdout);
    assert_eq!(report[2], "cluster fake: 38 of 39 stores live, 2 objects, 3 connections, 38 watch streams");
    let refused = "forbidden: forbidden: the user of the test cannot read this path (HTTP 403) (for ";
    assert!(report[16].contains(refused) && report[16].ends_with(", no data yet"), "{}", report[16]);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_command_connects_to_nothing_with_no_context_and_refuses_a_bad_command_line() {
    let server = FakeApiServer::start().await.expect("a server");
    let home = home("watch-list", &server);
    let run = home.run("watch", &[]).await;
    assert_eq!(run.status, Some(0), "stderr:\n{}", run.stderr);
    assert!(run.stdout.starts_with("1 context in 1 kubeconfig file:\n  fake  127.0.0.1:"), "{}", run.stdout);
    assert!(run.stdout.ends_with("Select a context with --context <name>, or each context with --all.\n"));

    for (arguments, problem) in [
        (&["--context", "prod"][..], "the kubeconfig files have no context `prod`\n\nusage: fresnel-cli watch"),
        (&["--for", "0"][..], "the value of --for is a number from 1 to 604800, not `0`\n\nusage: fresnel-cli watch"),
    ] {
        let run = home.run("watch", arguments).await;
        assert_eq!((run.status, run.stdout.as_str()), (Some(2), ""), "{arguments:?}");
        assert!(run.stderr.starts_with(problem), "{}", run.stderr);
    }
    assert_eq!(server.counters().connections, 0, "no run opened a connection");
}
