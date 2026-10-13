//! The commands of the UI: the selection and "Retry now".

use std::time::Duration;

use fresnel_core::app::{Command, CoreSettings, Health};
use fresnel_core::cluster::ClusterId;
use fresnel_core::journal::Level;
use fresnel_core::net::credential::CredentialState;
use fresnel_testkit::FakeApiServer;

use super::selection::until_saved;
use super::{App, KUBECONFIG, PLUGIN, home, row, states, write_kubeconfig};
use crate::bed::authority;
use crate::common::in_time;

#[tokio::test(flavor = "multi_thread")]
async fn a_selection_connects_a_cluster_and_closes_the_connections_of_one_that_leaves_it() {
    let (a, b) = (FakeApiServer::start().await.expect("a server"), FakeApiServer::start().await.expect("a server"));
    let home = home("select");
    write_kubeconfig(&home, "a", &[("a", a.addr(), &a, false), ("b", b.addr(), &b, false)]);
    let app = App::started(&home, CoreSettings::default());
    app.until_health("a", Health::Live).await;

    app.select("b", true);
    let two = app.until_health("b", Health::Live).await;
    assert_eq!(states(&two), [("a", true, Health::Live), ("b", true, Health::Live)]);
    assert_eq!((a.counters().connections, b.counters().connections), (3, 3), "one connection for each purpose");

    app.select("a", false);
    // A cluster that no kubeconfig file has: the command changes nothing.
    app.select("absent", true);
    let one = app.until_health("a", Health::Off).await;
    assert_eq!(states(&one), [("a", false, Health::Off), ("b", true, Health::Live)]);
    in_time("each connection of a closes", a.wait_for(|counters| counters.open_connections == 0)).await;
    assert_eq!(app.texts_of("a").last().map(String::as_str), Some("not selected: the connections close"));
    assert_eq!(b.counters().open_connections, 3, "the other cluster keeps its connections");

    // A cluster that is selected again connects again.
    app.select("a", true);
    app.until_health("a", Health::Live).await;
    assert_eq!(a.counters().connections, 6);
    until_saved(&home, &["a", "b"]).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn retry_now_runs_a_failed_plugin_again_at_once() {
    let server = FakeApiServer::start().await.expect("a server");
    let home = home("retry-plugin");
    home.write("state/mode", "fail").expect("the mode of the plugin");
    write_kubeconfig(&home, "fake", &[("fake", server.addr(), &server, true)]);
    let mut settings = CoreSettings::default();
    // With no retry by the user, the next run would start after 10 minutes.
    settings.net.credential.retry_first = Duration::from_secs(600);
    settings.net.credential.retry_max = Duration::from_secs(600);
    let app = App::started(&home, settings);

    let view = app.until_health("fake", Health::NoCredential).await;
    let failed = format!("credential plugin `{PLUGIN}`: stopped with exit status: 7");
    assert_eq!(row(&view, "fake").reason(), Some(failed.clone()));
    let credential = row(&view, "fake").status().and_then(|status| status.credential.clone()).expect("a credential");
    let CredentialState::Failed { failure, .. } = &credential.state else { panic!("{credential:?}") };
    assert_eq!(failure.detail(), "Error: access token is required (secret-text-of-the-plugin)");
    let entry = format!("no credential ({failed})");
    app.until_texts("fake", |texts| texts.contains(&entry)).await;
    let journal = app.journal();
    assert!(journal.contains(&(Level::Error, "fake".to_owned(), entry)), "{journal:?}");
    assert!(journal.iter().all(|(_, _, text)| !text.contains("secret-text")), "the journal goes to a log: {journal:?}");

    // The user repairs the plugin and asks for a retry.
    home.write("state/mode", "ok").expect("the mode of the plugin");
    app.core.send(Command::RetryNow(ClusterId::new("fake")));
    let retried = tokio::time::Instant::now();
    app.until_health("fake", Health::Live).await;
    assert!(retried.elapsed() < Duration::from_secs(10), "{:?}", retried.elapsed());
    assert_eq!(home.read("state/runs"), "run\nrun\n");
    let texts = app.until_texts("fake", |texts| texts.last().is_some_and(|text| text.starts_with("live: "))).await;
    assert!(texts.iter().any(|text| text.starts_with("retry now: ")), "{texts:?}");
    assert_eq!(texts.last().map(String::as_str), Some("live: a request got an answer on each channel"));
}

#[tokio::test(flavor = "multi_thread")]
async fn retry_now_loads_the_kubeconfig_again_for_a_cluster_whose_connect_did_not_start() {
    let server = FakeApiServer::start().await.expect("a server");
    let home = home("retry-entry");
    let address = server.addr();
    let broken = format!(
        "current-context: late
clusters:
- {{name: late, cluster: {{server: 'https://{address}', certificate-authority: absent.pem}}}}
users:
- {{name: token, user: {{token: static-token-of-the-test}}}}
contexts:
- {{name: late, context: {{cluster: late}}}}
- {{name: token, context: {{cluster: late, user: token}}}}
"
    );
    home.write(KUBECONFIG, &broken).expect("the kubeconfig");
    let app = App::started(&home, CoreSettings::default());

    let view = app.until_health("late", Health::Failed).await;
    let reason = row(&view, "late").reason().expect("a reason");
    assert!(reason.starts_with("cannot read the certificate authority "), "{reason}");
    assert_eq!(app.texts_of("late"), [format!("the connect did not start: {reason}")]);

    // An entry that the app does not support shows its reason before a connect and after it.
    let unsupported = "the user entry `token` has a static token; the app does not support that yet";
    assert_eq!(row(&view, "token").server.as_ref().expect_err("no server").to_string(), unsupported);
    assert_eq!((row(&view, "token").auth.clone(), row(&view, "token").health()), (None, Health::Off));
    app.select("token", true);
    let refused = app.until_health("token", Health::Failed).await;
    assert_eq!(row(&refused, "token").reason().as_deref(), Some(unsupported));
    assert_eq!(server.counters().connections, 0);

    // The user repairs the file and asks for a retry.
    write_kubeconfig(&home, "late", &[("late", address, &server, false)]);
    app.core.send(Command::RetryNow(ClusterId::new("late")));
    let repaired = app.until_health("late", Health::Live).await;
    assert_eq!(states(&repaired), [("late", true, Health::Live)], "the context `token` left the file");
}

/// A second fault during a recovery (design rule 5): the user repairs the kubeconfig entry
/// of one cluster and leaves a syntax error in the file. Each cluster of the file stays.
#[tokio::test(flavor = "multi_thread")]
async fn retry_now_with_a_kubeconfig_file_that_does_not_load_keeps_each_cluster_of_the_file() {
    let server = FakeApiServer::start().await.expect("a server");
    let home = home("retry-broken");
    let (address, good) = (server.addr(), format!("certificate-authority-data: {}", authority(&server)));
    let file = |late: &str| {
        format!(
            "current-context: live
clusters:
- {{name: live, cluster: {{server: 'https://{address}', {good}}}}}
- {{name: late, cluster: {{server: 'https://{address}', {late}}}}}
contexts:
- {{name: live, context: {{cluster: live}}}}
- {{name: late, context: {{cluster: late}}}}
"
        )
    };
    home.write(KUBECONFIG, &file("certificate-authority: absent.pem")).expect("the kubeconfig");
    let app = App::started(&home, CoreSettings::default());
    app.select("late", true);
    app.until_health("late", Health::Failed).await;
    app.until_health("live", Health::Live).await;

    home.write(KUBECONFIG, "clusters: [\n").expect("a file with a syntax error");
    app.core.send(Command::RetryNow(ClusterId::new("late")));
    let failures = |app: &App| app.texts_of("late").iter().filter(|text| text.starts_with("the connect did")).count();
    let kept = app.until("the retry ends with a second failure", |_| failures(&app) == 2).await;
    assert_eq!(states(&kept), [("live", true, Health::Live), ("late", true, Health::Failed)]);
    assert_eq!((server.counters().connections, server.counters().open_connections), (3, 3), "live keeps its channels");
    let warnings: Vec<String> =
        app.journal().into_iter().filter(|(level, ..)| *level == Level::Warning).map(|(.., text)| text).collect();
    assert!(warnings.iter().any(|text| text.contains(": the file is not a kubeconfig: line 1, ")), "{warnings:?}");
    assert!(warnings.contains(&"2 clusters of a file that does not load now stay as they were".to_owned()));

    // The second repair is complete.
    home.write(KUBECONFIG, &file(&good)).expect("the kubeconfig");
    app.core.send(Command::RetryNow(ClusterId::new("late")));
    let repaired = app.until_health("late", Health::Live).await;
    assert_eq!(states(&repaired), [("live", true, Health::Live), ("late", true, Health::Live)]);
    assert_eq!(server.counters().connections, 6, "live did not connect again");
}
