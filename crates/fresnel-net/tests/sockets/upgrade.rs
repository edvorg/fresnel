//! The upgrade connections on real sockets: the warm set, its test before use and its drop.

use std::time::{Duration, Instant};

use bytes::Bytes;
use fresnel_net::{Cause, Cluster, ClusterSpec, RequestKind, UpgradeStatus};
use fresnel_testkit::{FakeApiServer, Fault, FaultProxy};
use http::header::{CONNECTION, UPGRADE};
use http::{Request, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use crate::common::{Scratch, TestTasks, finder_env, in_time, plugin, spec, trust, until, until_live};

const EXEC: &str = "/api/v1/namespaces/default/pods/shell/exec?command=sh&stdin=true&stdout=true";

/// Sends an upgrade request, as exec does, and checks the raw stream: the fake server sends
/// each byte back.
async fn upgrade_and_echo(cluster: &Cluster) {
    let mut request = Request::get(EXEC).body(Bytes::new()).expect("a request");
    request.headers_mut().insert(CONNECTION, "Upgrade".parse().expect("a header"));
    request.headers_mut().insert(UPGRADE, "websocket".parse().expect("a header"));
    request.extensions_mut().insert(RequestKind::Upgrade);
    let response = cluster.send(request).await.expect("the answer to the upgrade");
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
    let mut stream =
        TokioIo::new(in_time("the raw stream", hyper::upgrade::on(response)).await.expect("the raw stream"));
    stream.write_all(b"keys of a terminal").await.expect("a write");
    let mut echo = [0; 18];
    in_time("the echo", stream.read_exact(&mut echo)).await.expect("the echo");
    assert_eq!(&echo, b"keys of a terminal");
}

async fn until_warm(cluster: &Cluster, what: &str, condition: impl Fn(&UpgradeStatus) -> bool) -> UpgradeStatus {
    until(cluster, what, |status| condition(&status.upgrade)).await.upgrade
}

#[tokio::test]
async fn an_upgrade_uses_a_warm_connection_and_the_set_fills_again() {
    let server = FakeApiServer::start().await.expect("a server");
    let tasks = TestTasks::new();
    let cluster = Cluster::start(&tasks.tasks, spec("warm", server.addr(), trust(&server))).expect("a cluster");
    until_live(&cluster).await;
    assert_eq!(server.counters().connections, 3, "no warm connection while no forward and no terminal is open");

    upgrade_and_echo(&cluster).await;
    assert_eq!(
        (cluster.status().upgrade.opened, server.counters().connections),
        (1, 4),
        "a cold upgrade pays for a connect"
    );

    // A terminal is open: the set holds 2 idle connections.
    let terminal = cluster.keep_upgrades_warm();
    until_warm(&cluster, "two warm connections", |upgrade| upgrade.warm == 2).await;
    assert_eq!(server.counters().connections, 6);
    upgrade_and_echo(&cluster).await;
    let status =
        until_warm(&cluster, "the set is full again", |upgrade| upgrade.warm == 2 && upgrade.reused == 1).await;
    assert_eq!((status.opened, status.holders), (1, 1), "the upgrade made no new connection: {status:?}");
    assert_eq!(server.counters().upgrades, 2);
    in_time("one new connection fills the set", server.wait_for(|counters| counters.connections == 7)).await;

    // A second terminal: one more idle connection. No terminal: none.
    let second = cluster.keep_upgrades_warm();
    until_warm(&cluster, "three warm connections", |upgrade| upgrade.warm == 3).await;
    drop((terminal, second));
    until_warm(&cluster, "no warm connection", |upgrade| upgrade.warm == 0 && upgrade.holders == 0).await;
}

/// The condition after a wake or after a NAT device lost its table: each open connection
/// is silent, and a new connection works.
#[tokio::test]
async fn a_silent_warm_connection_costs_its_short_test_and_not_the_limit_of_the_request() {
    let server = FakeApiServer::start().await.expect("a server");
    let proxy = FaultProxy::start(server.addr()).await.expect("a proxy");
    let tasks = TestTasks::new();
    let cluster = Cluster::start(&tasks.tasks, spec("silent-warm", proxy.addr(), trust(&server))).expect("a cluster");
    let _terminal = cluster.keep_upgrades_warm();
    until_warm(&cluster, "two warm connections", |upgrade| upgrade.warm == 2).await;

    proxy.silence_open();
    let started = Instant::now();
    upgrade_and_echo(&cluster).await;
    let needed = started.elapsed();
    assert!(Duration::from_secs(1) <= needed && needed < Duration::from_secs(4), "the upgrade needed {needed:?}");
    let status = cluster.status().upgrade;
    assert_eq!((status.reused, status.opened), (0, 1), "the silent connection was not used: {status:?}");
    assert!(status.dropped >= 2, "one failed test drops the set: {status:?}");
    until_warm(&cluster, "a new warm set", |upgrade| upgrade.warm == 2).await;
    upgrade_and_echo(&cluster).await;
    assert_eq!(cluster.status().upgrade.reused, 1);
}

#[tokio::test]
async fn a_wake_drops_the_warm_set_and_new_connections_replace_it() {
    let server = FakeApiServer::start().await.expect("a server");
    let tasks = TestTasks::new();
    let cluster = Cluster::start(&tasks.tasks, spec("wake", server.addr(), trust(&server))).expect("a cluster");
    until_live(&cluster).await;
    let _forward = cluster.keep_upgrades_warm();
    until_warm(&cluster, "two warm connections", |upgrade| upgrade.warm == 2).await;
    let before = server.counters().connections;

    cluster.signal().raise(Cause::Wake);
    let status = until_warm(&cluster, "a new warm set", |upgrade| upgrade.warm == 2 && upgrade.dropped == 2).await;
    assert_eq!(status.reused, 0);
    in_time("two new connections", server.wait_for(|counters| counters.connections == before + 2)).await;
    upgrade_and_echo(&cluster).await;
}

/// The token rule is also in the path of an upgrade: exec with a refused first token.
#[tokio::test]
async fn an_upgrade_with_a_refused_token_goes_out_again_with_a_new_token() {
    let server = FakeApiServer::start().await.expect("a server");
    server.add_fault(Fault::unauthorized().for_token("exec-1"));
    let tasks = TestTasks::new();
    let state = Scratch::new("plugin-upgrade");
    let spec = ClusterSpec {
        plugin: Some(plugin(&state, "exec")),
        env: finder_env(),
        ..spec("exec", server.addr(), trust(&server))
    };
    let cluster = Cluster::start(&tasks.tasks, spec).expect("a cluster");

    upgrade_and_echo(&cluster).await;
    let credential = cluster.status().credential.expect("a credential");
    assert_eq!((credential.plugin_runs, credential.proofs), (2, 0));
    let upgrades: Vec<_> =
        server.requests().into_iter().filter(|request| request.uri.path().ends_with("/exec")).collect();
    let tokens: Vec<_> = upgrades.iter().map(|request| request.bearer()).collect();
    assert_eq!(tokens, [Some("exec-1"), Some("exec-2")]);
    assert!(upgrades.iter().all(|request| request.version == http::Version::HTTP_11));
}
