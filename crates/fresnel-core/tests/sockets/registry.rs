//! The registry: the list of the clusters and the owner of their connections.

use std::path::Path;

use fresnel_core::cluster::{ClusterId, LinkState, Registry};
use fresnel_core::kubeconfig;
use fresnel_core::net::{Cause, ChannelState, ErrorClass, NetSettings, Protocol};
use fresnel_testkit::FakeApiServer;
use k8s_openapi::apimachinery::pkg::version::Info;
use tokio::runtime::Handle;

use crate::bed::{Bed, CONTEXT, Folder, authority, get, kubeconfig as kubeconfig_text};
use crate::common::{TestTasks, connect, in_time, registry, until_live};

fn states(registry: &Registry) -> Vec<(String, &'static str)> {
    let state = |state: &LinkState| match state {
        LinkState::Off => "off",
        LinkState::On(_) => "on",
        LinkState::Failed(_) => "failed",
    };
    registry.snapshot().iter().map(|view| (view.id.to_string(), state(&view.state))).collect()
}

fn reload(registry: &Registry, path: &Path) {
    registry.set_contexts(kubeconfig::load(&[path.to_owned()]).contexts);
}

#[tokio::test]
async fn a_cluster_connects_one_time_and_a_disconnect_closes_its_connections() {
    let bed = Bed::start(NetSettings::default()).await;
    let id = ClusterId::new(CONTEXT);
    assert_eq!(states(&bed.registry), [(CONTEXT.to_owned(), "on")]);
    let LinkState::On(status) = &bed.registry.snapshot()[0].state else { panic!("the cluster is connected") };
    assert_eq!((status.name.as_str(), &status.requests.channels[0].state), (CONTEXT, &ChannelState::Live));

    let again = connect(&bed.registry, CONTEXT);
    let _: Info = again.gateway.get(get("/version")).await.expect("the version");
    assert!(bed.registry.link(&id).is_some());
    assert_eq!(bed.server.counters().connections, 3, "a second connect gives the same connections");
    let probe = bed.registry.probe(&id).expect("a probe for the endpoint of the cluster");
    assert_eq!(probe.hedged(Protocol::Http2).await.expect("a connection").attempts, 1);

    let mut raised = bed.link.net.signal().listen();
    bed.registry.raise(Cause::Wake);
    assert_eq!(in_time("the signal of the cluster", raised.raised()).await, Cause::Wake);

    // The connections close when the registry and each other owner dropped the link.
    let Bed { link, registry, tasks, server, folder } = bed;
    registry.disconnect(&id);
    assert!(registry.link(&id).is_none());
    assert_eq!(states(&registry), [(CONTEXT.to_owned(), "off")]);
    drop((link, again));
    in_time("each connection closes", server.wait_for(|counters| counters.open_connections == 0)).await;
    assert_eq!(tasks.tasks.snapshot(), [], "each task of the cluster stopped");
    drop(folder);
}

#[tokio::test]
async fn a_new_load_keeps_a_cluster_with_the_same_context_and_closes_one_that_changed() {
    let Bed { link, registry, tasks, server, folder } = Bed::start(NetSettings::default()).await;
    drop(link);
    let other = FakeApiServer::start().await.expect("a server");
    let two = kubeconfig_text(&[(CONTEXT, server.addr(), &server), ("other", other.addr(), &other)]);
    let path = folder.write("config", &two);

    reload(&registry, &path);
    assert_eq!(states(&registry), [(CONTEXT.to_owned(), "on"), ("other".to_owned(), "off")]);
    assert_eq!(server.counters().connections, 3, "the cluster kept its connections");
    until_live(&connect(&registry, "other")).await;

    // The context `bed` gets a namespace, and the context `other` leaves the file.
    let changed = kubeconfig_text(&[(CONTEXT, server.addr(), &server)])
        .replace("{cluster: bed}", "{cluster: bed, namespace: apps}");
    reload(&registry, &folder.write("config", &changed));
    assert_eq!(states(&registry), [(CONTEXT.to_owned(), "off")]);
    for closed in [&server, &other] {
        in_time("each connection closes", closed.wait_for(|counters| counters.open_connections == 0)).await;
    }
    assert_eq!(registry.snapshot()[0].context.namespace.as_deref(), Some("apps"));
    assert_eq!(tasks.tasks.snapshot(), []);
}

#[tokio::test]
async fn a_cluster_that_cannot_connect_shows_its_error_and_the_other_cluster_works() {
    let server = FakeApiServer::start().await.expect("a server");
    let folder = Folder::new("failed");
    let authority = authority(&server);
    let address = server.addr();
    let text = format!(
        "clusters:
- {{name: good, cluster: {{server: 'https://{address}', certificate-authority-data: {authority}}}}}
- {{name: no-file, cluster: {{server: 'https://{address}', certificate-authority: absent.pem}}}}
- {{name: not-pem, cluster: {{server: 'https://{address}', certificate-authority-data: QUJD}}}}
users:
- {{name: token, user: {{token: secret-token}}}}
contexts:
- {{name: good, context: {{cluster: good}}}}
- {{name: no-file, context: {{cluster: no-file}}}}
- {{name: not-pem, context: {{cluster: not-pem}}}}
- {{name: token, context: {{cluster: good, user: token}}}}
"
    );
    let tasks = TestTasks::new(&Handle::current());
    let registry = registry(&tasks, &Handle::current(), &folder.write("config", &text), NetSettings::default());
    let error = |context: &str| {
        let error = registry.connect(&ClusterId::new(context)).expect_err("the cluster cannot connect");
        assert_eq!(error.class(), ErrorClass::Invalid, "{error}");
        error.to_string()
    };
    assert!(error("no-file").starts_with("cannot read the certificate authority "), "{}", error("no-file"));
    assert_eq!(error("not-pem"), "the certificate authority of the kubeconfig holds no certificate");
    assert_eq!(error("token"), "the user entry `token` has a static token; the app does not support that yet");
    assert_eq!(error("gone"), "the kubeconfig files have no context `gone`");
    until_live(&connect(&registry, "good")).await;

    let expected = [("good", "on"), ("no-file", "failed"), ("not-pem", "failed"), ("token", "failed")];
    assert_eq!(states(&registry), expected.map(|(name, state)| (name.to_owned(), state)));
    let LinkState::Failed(shown) = &registry.snapshot()[3].state else { panic!("the state of `token`") };
    assert_eq!(shown.to_string(), error("token"), "each cluster keeps its own error");
    assert_eq!(server.counters().connections, 3, "only the good cluster opened connections");
}

/// The plugin of a kubeconfig user: a command with a path that starts from the folder of
/// the kubeconfig file, with the variables and the cluster data that the entry asks for.
#[cfg(unix)]
#[tokio::test]
async fn the_plugin_of_the_kubeconfig_gives_the_token_of_each_request() {
    let server = FakeApiServer::start().await.expect("a server");
    let folder = Folder::new("plugin");
    std::fs::create_dir(folder.path().join("bin")).expect("a folder");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fresnel-net/tests/fixtures/fake-plugin");
    std::os::unix::fs::symlink(fixture, folder.path().join("bin/plugin")).expect("a link to the plugin of the tests");
    let (address, authority, state) = (server.addr(), authority(&server), folder.path().display());
    let text = format!(
        "clusters:
- {{name: c, cluster: {{server: 'https://{address}', certificate-authority-data: {authority}}}}}
users:
- name: u
  user:
    exec:
      apiVersion: client.authentication.k8s.io/v1
      command: bin/plugin
      provideClusterInfo: true
      env: [{{name: FAKE_PLUGIN_STATE, value: '{state}'}}, {{name: FAKE_PLUGIN_TOKEN, value: core}}]
contexts:
- {{name: plugin, context: {{cluster: c, user: u}}}}
"
    );
    let tasks = TestTasks::new(&Handle::current());
    let registry = registry(&tasks, &Handle::current(), &folder.write("config", &text), NetSettings::default());
    let link = connect(&registry, "plugin");
    let _: serde_json::Value = link.gateway.get(get("/api/v1/namespaces")).await.expect("an answer");

    until_live(&link).await;
    // A channel can send its first light request before the plugin gave the token.
    let tokens: Vec<_> =
        server.requests().iter().map(|seen| (seen.uri.path().to_owned(), seen.bearer().map(str::to_owned))).collect();
    assert!(tokens.contains(&("/api/v1/namespaces".to_owned(), Some("core-1".to_owned()))), "{tokens:?}");
    assert!(tokens.iter().all(|(_, token)| token.as_deref().is_none_or(|token| token == "core-1")), "{tokens:?}");
    assert_eq!(link.net.status().credential.expect("a credential").plugin_runs, 1);
    let exec_info: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(folder.path().join("exec-info")).expect("the input"))
            .expect("JSON");
    assert_eq!(exec_info["apiVersion"], "client.authentication.k8s.io/v1");
    assert_eq!(exec_info["spec"]["cluster"]["server"], format!("https://{address}"));
    assert!(exec_info["spec"]["cluster"]["certificate-authority-data"].as_str().is_some_and(|data| data.len() > 100));
}

/// Two parts of the app can connect one cluster at the same moment. The cluster starts one
/// time for each round: a second start would run the plugin of the user a second time.
#[cfg(unix)]
#[test]
fn threads_that_connect_one_cluster_at_the_same_moment_start_it_one_time() {
    use fresnel_core::net::ClusterStatus;
    use fresnel_core::net::credential::CredentialState;

    use crate::common::until;

    const THREADS: usize = 8;
    const ROUNDS: usize = 25;
    let runtime =
        tokio::runtime::Builder::new_multi_thread().worker_threads(4).enable_all().build().expect("a runtime");
    let server = runtime.block_on(FakeApiServer::start()).expect("a server");
    let folder = Folder::new("race");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fresnel-net/tests/fixtures/fake-plugin");
    let (address, authority, state) = (server.addr(), authority(&server), folder.path().display());
    let text = format!(
        "clusters:
- {{name: c, cluster: {{server: 'https://{address}', certificate-authority-data: {authority}}}}}
users:
- name: u
  user:
    exec:
      command: {}
      env: [{{name: FAKE_PLUGIN_STATE, value: '{state}'}}]
contexts:
- {{name: race, context: {{cluster: c, user: u}}}}
",
        fixture.display()
    );
    let tasks = TestTasks::new(runtime.handle());
    let registry = registry(&tasks, runtime.handle(), &folder.write("config", &text), NetSettings::default());
    let id = ClusterId::new("race");

    for round in 1..=ROUNDS {
        let start = std::sync::Barrier::new(THREADS);
        let links: Vec<_> = std::thread::scope(|scope| {
            let connect_at_start = || {
                start.wait();
                connect(&registry, "race")
            };
            let threads: Vec<_> = (0..THREADS).map(|_| scope.spawn(connect_at_start)).collect();
            threads.into_iter().map(|thread| thread.join().expect("the thread connects")).collect()
        });
        runtime.block_on(until_live(&links[0]));
        let has_token = |status: &ClusterStatus| {
            status
                .credential
                .as_ref()
                .is_some_and(|credential| matches!(credential.state, CredentialState::Valid { .. }))
        };
        let status = runtime.block_on(until(&links[0], "the plugin gave a token", has_token));
        assert_eq!(status.credential.expect("a credential").plugin_runs, 1);
        let runs = std::fs::read_to_string(folder.path().join("runs")).expect("the runs of the plugin");
        assert_eq!(runs.lines().count(), round, "one plugin process for each round");
        assert_eq!(server.counters().connections, 3 * round as u64, "three channels for each round");
        registry.disconnect(&id);
        drop(links);
        runtime.block_on(in_time("each connection closes", server.wait_for(|counters| counters.open_connections == 0)));
        assert_eq!(tasks.tasks.snapshot(), [], "each task of the cluster stopped");
    }
}

/// The UI thread connects a cluster. It has no runtime context.
#[test]
fn a_thread_with_no_runtime_connects_a_cluster() {
    let runtime =
        tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().expect("a runtime");
    let server = runtime.block_on(FakeApiServer::start()).expect("a server");
    let folder = Folder::new("no-runtime");
    let path = folder.write("config", &kubeconfig_text(&[("ui", server.addr(), &server)]));
    let tasks = TestTasks::new(runtime.handle());
    let registry = registry(&tasks, runtime.handle(), &path, NetSettings::default());
    assert!(Handle::try_current().is_err(), "this thread has no runtime context");

    let link = connect(&registry, "ui");
    let version: Info = runtime.block_on(link.gateway.get(get("/version"))).expect("the version");
    assert_eq!(version.git_version, "v1.33.5+fake");
    drop((link, registry));
}
