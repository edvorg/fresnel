//! The start of the core and its stop.

use std::time::Duration;

use fresnel_core::app::{CoreSettings, Health, Startup};
use fresnel_core::kubeconfig::AuthKind;
use fresnel_core::net::credential::CredentialState;
use fresnel_testkit::FakeApiServer;
use http::Method;

use super::{App, PLUGIN, home, row, states, write_kubeconfig};
use crate::common::{LIMIT, in_time};

/// The acceptance of milestone M0 for the core: the window opens before the first network
/// call, and the app starts from Finder and finds the kubeconfig and the plugin.
#[tokio::test(flavor = "multi_thread")]
async fn the_core_does_nothing_before_its_start_and_then_connects_with_the_environment_of_finder() {
    let server = FakeApiServer::start().await.expect("a server");
    let other = FakeApiServer::start().await.expect("a server");
    let home = home("finder");
    write_kubeconfig(&home, "fake", &[("fake", server.addr(), &server, true), ("other", other.addr(), &other, false)]);
    let app = App::new(&home, CoreSettings::default());

    // Only a check that nothing occurs waits a fixed time.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let idle = app.core.view();
    assert_eq!((idle.startup, idle.clusters.len(), idle.journal.total), (Startup::Waiting, 0, 0));
    assert_eq!((server.counters().connections, home.read("state/runs").as_str()), (0, ""));

    app.start();
    app.start();
    let view = app.until_health("fake", Health::Live).await;
    assert_eq!(states(&view), [("fake", true, Health::Live), ("other", false, Health::Off)]);
    assert_eq!(view.shell_problem, None);
    let fake = row(&view, "fake");
    assert_eq!(fake.server.as_ref().expect("a server"), &server.addr().to_string());
    assert_eq!((fake.auth.clone(), fake.reason()), (Some(AuthKind::Plugin(PLUGIN.to_owned())), None));
    assert!(fake.last_answer_age().is_some_and(|age| age < LIMIT), "{:?}", fake.last_answer_age());
    let credential = fake.status().and_then(|status| status.credential.clone()).expect("a credential");
    assert!(matches!(credential.state, CredentialState::Valid { .. }), "{credential:?}");
    assert_eq!((credential.plugin_runs, home.read("state/runs").as_str()), (1, "run\n"), "one run of the plugin");
    assert_eq!(row(&view, "other").last_answer_age(), None);

    // The core changes nothing in a cluster: each request is a read.
    let requests = server.requests();
    assert!(requests.iter().any(|seen| seen.uri.path() == "/version"), "the light request of a channel");
    assert!(requests.iter().all(|seen| seen.method == Method::GET), "{requests:?}");
    assert_eq!(other.counters().connections, 0, "a cluster that is not selected gets no connection");

    let texts = app.texts_of("app");
    assert!(texts[0].starts_with("the login shell /") && texts[0].contains(" gave its environment in "), "{texts:?}");
    assert_eq!(texts[1..], ["read 2 contexts from 1 kubeconfig file"]);
    let of_cluster = app.until_texts("fake", |texts| texts.len() >= 2).await;
    assert_eq!(of_cluster, ["the channels connect", "live: a request got an answer on each channel"]);
}

/// The quit sequence of the core: no connection and no plugin process stays.
#[tokio::test(flavor = "multi_thread")]
async fn the_stop_closes_each_connection_and_ends_a_plugin_that_runs() {
    let (plain, login) =
        (FakeApiServer::start().await.expect("a server"), FakeApiServer::start().await.expect("a server"));
    let home = home("stop");
    home.write("state/mode", "hang").expect("the mode of the plugin");
    write_kubeconfig(&home, "plain", &[("plain", plain.addr(), &plain, false), ("login", login.addr(), &login, true)]);
    let app = App::started(&home, CoreSettings::default());
    app.select("login", true);
    app.until_health("plain", Health::Live).await;
    let child = async {
        while home.read("state/child").is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        (home.read("state/pid"), home.read("state/child"))
    };
    let (plugin, child) = in_time("the plugin started its child process", child).await;
    assert!(process_exists(&plugin) && process_exists(&child), "the plugin and its child process run");

    app.core.stop();
    in_time("each connection closes", plain.wait_for(|counters| counters.open_connections == 0)).await;
    in_time("each connection closes", login.wait_for(|counters| counters.open_connections == 0)).await;
    let gone = async {
        while process_exists(&plugin) || process_exists(&child) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    in_time("the plugin and its child process are gone", gone).await;
    let stopped = app.core.view();
    assert_eq!((stopped.startup, stopped.clusters.len()), (Startup::Ready, 0));

    // A stopped core takes no command and does not start again.
    app.select("plain", true);
    app.start();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!((plain.counters().open_connections, app.core.view().clusters.len()), (0, 0));
}

/// True while a process with this id exists.
fn process_exists(id: &str) -> bool {
    let id: i32 = id.trim().parse().expect("a process id");
    nix::sys::signal::kill(nix::unistd::Pid::from_raw(id), None).is_ok()
}
