//! The network numbers of the acceptance of milestone M0 (plan section 15).

use std::time::{Duration, Instant, SystemTime};

use fresnel_net::{ChannelState, Cluster, RequestKind};
use fresnel_testkit::{FakeApiServer, FaultProxy, ProxyMode};
use http_body_util::BodyExt as _;

use crate::common::{TestTasks, fetch, first_channels, in_time, send, spec, trust, until, until_live};

const WATCH: &str = "/api/v1/pods?watch=true";

/// "A silent connection shows in 20 seconds or less."
#[tokio::test]
async fn a_silent_connection_shows_in_20_seconds_or_less() {
    let server = FakeApiServer::start().await.expect("a server");
    let proxy = FaultProxy::start(server.addr()).await.expect("a proxy");
    let tasks = TestTasks::new();
    let cluster = Cluster::start(&tasks.tasks, spec("silent", proxy.addr(), trust(&server))).expect("a cluster");
    until_live(&cluster).await;
    let mut events = send(&cluster, RequestKind::Watch, WATCH).await.expect("a watch").into_body();
    assert_eq!(server.send_event("/api/v1/pods", r#"{"type":"ADDED"}"#), 1);
    assert!(in_time("the event", events.frame()).await.expect("a frame").is_ok());

    // Each open connection carries no more bytes and gives no error: a path through a NAT
    // device that lost its table. A new connection works.
    proxy.silence_open();
    let silent_at = Instant::now();
    let status = until(&cluster, "each channel lost its connection", |status| {
        first_channels(status).iter().all(|channel| channel.connections >= 2)
    })
    .await;
    let shown_after = silent_at.elapsed();
    println!("the silent connections showed after {shown_after:?}");
    assert!(shown_after <= Duration::from_secs(20), "the silent connections showed after {shown_after:?}");
    assert!(
        shown_after >= Duration::from_secs(5),
        "nothing but a time limit can show a silent connection: {shown_after:?}"
    );
    for channel in first_channels(&status) {
        let error = channel.last_error.as_deref().expect("the cause of the loss");
        assert!(error.contains("keep-alive timed out") || error.contains("no answer to the light request"), "{error}");
    }
    let ended = in_time("the end of the watch", events.frame()).await.expect("the end of the stream");
    assert!(ended.is_err(), "the watch of the silent connection ends with an error");

    // The channels connected again by themselves. Nothing asked for it.
    until_live(&cluster).await;
    assert_eq!(fetch(&cluster, RequestKind::Get, "/version").await.expect("an answer").0, 200);
    assert!(send(&cluster, RequestKind::Watch, WATCH).await.is_ok());
    assert_eq!((proxy.task_failures(), server.task_failures()), (Vec::<String>::new(), Vec::new()));
}

/// "With 12 percent held connections, fewer than 1 percent of connects take 1 second or more."
#[tokio::test]
async fn with_12_percent_held_connections_fewer_than_1_percent_of_connects_take_1_second() {
    const CLUSTERS: usize = 70;
    // Few clusters start at one time, so that the processor load of the test adds little to each time.
    const AT_ONE_TIME: usize = 5;
    let server = FakeApiServer::start().await.expect("a server");
    let proxy = FaultProxy::start(server.addr()).await.expect("a proxy");
    // A held connection gets no answer for 1 to 20 s. Thus only a second attempt can be fast.
    proxy.set(ProxyMode::HoldNew { probability: 0.12, min: Duration::from_secs(1), max: Duration::from_secs(20) });
    let tasks = TestTasks::new();
    let (address, tls) = (proxy.addr(), trust(&server));

    let mut times = Vec::new();
    let (mut connects, mut attempts) = (0, 0);
    for batch in 0..CLUSTERS / AT_ONE_TIME {
        let started = (0..AT_ONE_TIME).map(|number| {
            let name = format!("held-{batch}-{number}");
            let (tasks, tls) = (&tasks.tasks, tls.clone());
            async move {
                let start = SystemTime::now();
                let cluster = Cluster::start(tasks, spec(&name, address, tls)).expect("a cluster");
                (start, until_live(&cluster).await)
            }
        });
        for (start, status) in futures_util::future::join_all(started).await {
            // Each channel made one connect. It is live one small request after that connect.
            for channel in first_channels(&status) {
                assert_eq!((&channel.state, channel.connect_attempts), (&ChannelState::Live, 1));
                times.push(channel.since.duration_since(start).expect("live after the start"));
            }
            connects += status.connect.connects;
            attempts += status.connect.attempts;
        }
    }

    times.sort_unstable();
    let slow = times.iter().filter(|time| **time >= Duration::from_secs(1)).count();
    let held = proxy.counters().held;
    println!(
        "{} connects, {attempts} attempts, {held} held; median {:?}, p99 {:?}, longest {:?}; {slow} of 1 s or more",
        times.len(),
        times[times.len() / 2],
        times[times.len() * 99 / 100],
        times[times.len() - 1]
    );
    assert_eq!((times.len(), connects), (3 * CLUSTERS, 3 * CLUSTERS as u64));
    assert!(held >= 15, "the proxy held only {held} connections: the fault was not present");
    assert!(attempts > connects, "no connect started a second attempt");
    assert!(slow * 100 < times.len(), "{slow} of {} connects took 1 s or more", times.len());
}

/// "Eight resets in one minute: each recovery in 5 seconds or less."
#[tokio::test]
async fn eight_resets_in_one_minute_each_recover_in_5_seconds_or_less() {
    const WATCHES: usize = 5;
    let server = FakeApiServer::start().await.expect("a server");
    let proxy = FaultProxy::start(server.addr()).await.expect("a proxy");
    let tasks = TestTasks::new();
    let cluster = Cluster::start(&tasks.tasks, spec("resets", proxy.addr(), trust(&server))).expect("a cluster");
    until_live(&cluster).await;
    let open_watches = || async {
        let watches = (0..WATCHES).map(|_| send(&cluster, RequestKind::Watch, WATCH));
        futures_util::future::try_join_all(watches).await
    };
    let mut watches = open_watches().await.expect("watches");

    let first_reset = Instant::now();
    let mut recoveries = Vec::new();
    for reset in 1..=8_u64 {
        // The pause between two resets is the plan of the fault, not a wait for the code.
        tokio::time::sleep(Duration::from_secs(3)).await;
        in_time("the reset", proxy.reset_all()).await;
        let reset_at = Instant::now();
        for watch in watches.drain(..) {
            let ended = in_time("the end of a watch", watch.into_body().frame()).await;
            assert!(ended.is_none_or(|frame| frame.is_err()), "a watch of the reset connection ends");
        }
        // The recovery is complete when new watches run and a request gets its answer.
        watches = in_time("new watches", open_watches()).await.expect("watches after the reset");
        assert_eq!(fetch(&cluster, RequestKind::Get, "/version").await.expect("an answer").0, 200);
        recoveries.push(reset_at.elapsed());

        let status = until_live(&cluster).await;
        for channel in first_channels(&status) {
            assert_eq!(channel.connections, reset + 1, "one new connection for each reset: {channel:?}");
        }
    }
    println!("recovery after each reset: {recoveries:?}");
    assert!(first_reset.elapsed() < Duration::from_secs(60), "eight resets in one minute");
    for (number, recovery) in recoveries.iter().enumerate() {
        assert!(*recovery <= Duration::from_secs(5), "reset {}: recovery in {recovery:?}", number + 1);
    }
    in_time(
        "the server saw the end of the old watches",
        server.wait_for(|counters| counters.open_watches == WATCHES as u64),
    )
    .await;
    assert_eq!(proxy.counters().reset, 8 * 3, "three connections for each reset");
}

/// A host name can have a dead address before the good one. On most systems `localhost`
/// gives `::1` first, and the fake server listens on 127.0.0.1 only.
#[tokio::test]
async fn a_host_with_a_dead_address_connects_on_its_next_address() {
    let server = FakeApiServer::start().await.expect("a server");
    let tasks = TestTasks::new();
    let mut spec = spec("two-addresses", server.addr(), trust(&server));
    spec.endpoint.host = "localhost".to_owned();
    let cluster = Cluster::start(&tasks.tasks, spec).expect("a cluster");
    let connect = until_live(&cluster).await.connect;
    assert_eq!(fetch(&cluster, RequestKind::Get, "/version").await.expect("an answer").0, 200);
    assert!(connect.addresses.contains(&server.addr()), "{connect:?}");
    // A system with one address for `localhost` cannot show the dead address.
    if connect.addresses.first() != Some(&server.addr()) {
        assert!(connect.attempts >= 2 * connect.connects, "each connect starts at the dead address: {connect:?}");
    }
}

/// The UI thread starts a cluster. It has no runtime context.
#[test]
fn a_thread_with_no_runtime_starts_a_cluster() {
    let runtime =
        tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().expect("a runtime");
    let server = runtime.block_on(FakeApiServer::start()).expect("a server");
    let tasks = fresnel_net::task::Tasks::new(runtime.handle().clone(), |report| panic!("a task ended: {report}"));
    assert!(tokio::runtime::Handle::try_current().is_err(), "this thread has no runtime context");
    let cluster = Cluster::start(&tasks, spec("no-context", server.addr(), trust(&server))).expect("a cluster");
    let _terminal = cluster.keep_upgrades_warm();
    let status = runtime.block_on(until_live(&cluster));
    assert_eq!(status.name, "no-context");
    assert_eq!(runtime.block_on(fetch(&cluster, RequestKind::Get, "/version")).expect("an answer").0, 200);
}
