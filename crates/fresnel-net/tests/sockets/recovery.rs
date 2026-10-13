//! A second fault during a recovery. The acceptance lines of milestone M0 name one fault at
//! a time.

use std::time::{Duration, Instant};

use fresnel_net::{Cause, ChannelState, Cluster, RequestKind};
use fresnel_testkit::{FakeApiServer, FaultProxy, ProxyMode};
use http_body_util::BodyExt as _;

use crate::common::{TestTasks, fetch, first_channels, in_time, send, spec, trust, until, until_live};

const WATCH: &str = "/api/v1/pods?watch=true";
const MINUTE: Duration = Duration::from_secs(60);

/// The open connections become silent, and then each new connection gets no answer. A
/// connect attempt of that time stays lost when the network returns, as a SYN packet that
/// nobody sends again soon. Thus only an attempt that starts later can connect, and the
/// test measures the wait for it at a bad moment: each attempt slot is in use.
#[tokio::test]
async fn after_an_outage_with_lost_connects_each_channel_is_live_5_seconds_after_the_network_returns() {
    let server = FakeApiServer::start().await.expect("a server");
    let proxy = FaultProxy::start(server.addr()).await.expect("a proxy");
    let tasks = TestTasks::new();
    let cluster = Cluster::start(&tasks.tasks, spec("outage", proxy.addr(), trust(&server))).expect("a cluster");
    until_live(&cluster).await;
    let mut events = send(&cluster, RequestKind::Watch, WATCH).await.expect("a watch").into_body();

    proxy.silence_open();
    proxy.set(ProxyMode::HoldNew { probability: 1.0, min: MINUTE, max: MINUTE });
    cluster.signal().raise(Cause::NetworkChange);
    until(&cluster, "each channel lost its connection and connects again", |status| {
        first_channels(status).iter().all(|channel| channel.connect_attempts >= 2)
    })
    .await;
    let ended = in_time("the end of the watch", events.frame()).await;
    assert!(ended.is_none_or(|frame| frame.is_err()), "the watch of the silent connection ends");

    // The attempts of a connect start in its first second, and each one has a limit of 3 s.
    // After 4 s, the second set of attempts is in progress, and each of them is lost.
    tokio::time::sleep(Duration::from_secs(4)).await;
    let status = cluster.status();
    for channel in first_channels(&status) {
        assert!(channel.state != ChannelState::Live && channel.connections == 1, "no connection: {channel:?}");
    }
    assert!(proxy.counters().held >= 3 * 5, "each connect made a full set of attempts: {:?}", proxy.counters());

    // The network returns. Nothing tells the cluster: no wake and no network change.
    proxy.set(ProxyMode::Pass);
    let returned_at = Instant::now();
    let status = until(&cluster, "each channel is live on a new connection", |status| {
        first_channels(status).iter().all(|channel| channel.state == ChannelState::Live && channel.connections == 2)
    })
    .await;
    let recovery = returned_at.elapsed();
    println!("live {recovery:?} after the network returned, with {} lost connect attempts", proxy.counters().held);
    assert!(recovery <= Duration::from_secs(5), "the channels were live {recovery:?} after the network returned");
    for channel in first_channels(&status) {
        assert_eq!(channel.connect_attempts, 2, "one connect carried the full outage: {channel:?}");
    }
    assert_eq!(fetch(&cluster, RequestKind::Get, "/version").await.expect("an answer").0, 200);
    assert!(send(&cluster, RequestKind::Watch, WATCH).await.is_ok());
    assert_eq!((proxy.task_failures(), server.task_failures()), (Vec::<String>::new(), Vec::new()));
}
