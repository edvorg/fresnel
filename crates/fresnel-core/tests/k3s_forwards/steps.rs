//! The steps of the forwards test on k3s.

use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime};

use fresnel_core::forward::ForwardRule;
use fresnel_core::kinds::Kind;
use fresnel_testkit::ProxyMode;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpStream;
use tokio::time::{Instant, sleep, timeout};

use crate::bed::{Bed, forward, page, rule};
use crate::gaps::longest_hidden_failure;

const SECOND: Duration = Duration::from_secs(1);
/// The bytes of the upload: 200 MB (plan section 15, M5).
const UPLOAD: usize = 200 * 1024 * 1024;

/// Waits until the page of a local address comes, and gives the time of the wait.
async fn until_page(address: SocketAddr, word: &str, limit: Duration) -> Duration {
    let started = Instant::now();
    loop {
        match page(address).await {
            Ok(body) if body == word => return started.elapsed(),
            other => assert!(started.elapsed() < limit, "the page {word} in {limit:?}: {other:?}"),
        }
        sleep(SECOND / 10).await;
    }
}

/// Plan section 12.3: Pod, Service, Deployment, StatefulSet, DaemonSet, ReplicaSet,
/// ReplicationController and Job, each to its own pod. A Service takes the target port of
/// its pod by name. Gives each forward with its address.
pub(crate) async fn each_kind_of_target_reaches_a_pod_of_its_object(bed: &Bed) -> Vec<(ForwardRule, SocketAddr)> {
    let targets = [
        (rule(Kind::Pod, "web", 8080), "Pod"),
        (rule(Kind::Service, "svc", 80), "Deployment"),
        (rule(Kind::Deployment, "deploy", 8080), "Deployment"),
        (rule(Kind::StatefulSet, "stateful", 8080), "StatefulSet"),
        (rule(Kind::DaemonSet, "daemon", 8080), "DaemonSet"),
        (rule(Kind::ReplicaSet, "replicas", 8080), "ReplicaSet"),
        (rule(Kind::ReplicationController, "controller", 8080), "ReplicationController"),
        (rule(Kind::Job, "job", 8080), "Job"),
    ];
    let mut forwards = Vec::new();
    for (rule, word) in targets {
        let address = bed.active(&rule).await;
        assert_eq!(page(address).await.as_deref(), Ok(word), "{rule}");
        let view = bed.core.view();
        let target = forward(&view, &rule).and_then(|row| row.view.target.clone()).expect("a target pod");
        println!("{rule}: pod {} port {} on {address}", target.pod.name, target.port);
        forwards.push((rule, address));
    }
    forwards
}

/// Plan section 15, M5: a forward recovers 5 s or less after its pod restarts. The pod of the
/// Deployment goes; its new pod is ready after its start, and from then on the forward to
/// the Deployment and the one to its Service give the page again in 5 s or less.
pub(crate) async fn a_forward_works_again_5_s_or_less_after_its_pod_restarts(bed: &Bed) {
    let (deployment, service) = (rule(Kind::Deployment, "deploy", 8080), rule(Kind::Service, "svc", 80));
    let (to_deployment, to_service) = (bed.active(&deployment).await, bed.active(&service).await);
    let old = forward(&bed.core.view(), &deployment).and_then(|row| row.view.target.clone()).expect("a pod");
    bed.kubectl(&["delete", "pod", &old.pod.name, "--wait=false"]).await;
    bed.kubectl(&["rollout", "status", "deployment/deploy", "--timeout=180s"]).await;
    let ready = Instant::now();
    let (one, two) = tokio::join!(
        until_page(to_deployment, "Deployment", 30 * SECOND),
        until_page(to_service, "Deployment", 30 * SECOND)
    );
    let recovered = ready.elapsed();
    println!("the forwards gave the page {one:?} and {two:?} after the new pod was ready");
    assert!(recovered <= 5 * SECOND, "the forwards worked again {recovered:?} after the new pod was ready");
    // The store of the core gets the readiness of the new pod a moment after kubectl; until
    // then, the forward takes the pod because it runs.
    let target =
        |view: &fresnel_core::app::CoreView| forward(view, &deployment).and_then(|row| row.view.target.clone());
    let new_ready =
        |view: &fresnel_core::app::CoreView| target(view).is_some_and(|new| new.ready && new.pod != old.pod);
    bed.until("the new ready pod is the target", new_ready).await;
}

/// Plan section 12.3: "degraded" when each of the last connections failed. The node starts
/// the data channel of a tunnel before it dials the port in the pod, and then writes the
/// refusal: each tunnel to a port that nothing listens on opens and fails, so an open alone
/// may end no run of failures.
pub(crate) async fn a_forward_to_a_closed_port_degrades_after_three_failed_tunnels(bed: &Bed) {
    let closed = rule(Kind::Pod, "web", 9999);
    let address = bed.active(&closed).await;
    for client in 1..=3 {
        assert_eq!(end_seen(address).await, Ok("reset"), "client {client}");
    }
    let reset =
        |view: &fresnel_core::app::CoreView| forward(view, &closed).is_some_and(|row| row.view.connections.reset == 3);
    let view = bed.until("three failed tunnels", reset).await;
    let state = forward(&view, &closed).map(|row| row.view.state.to_string()).expect("the forward");
    println!("{closed}: {state}");
    assert!(state.starts_with("degraded: each of the last 3 tunnels failed: the node refused a tunnel: "), "{state}");
    assert!(state.contains("9999") && state.contains("connection refused"), "{state}");
}

/// Plan section 15, M5: an upload of 200 MB to a slow pod completes. The pod reads nothing
/// for 30 s: the write waits, and the path of the cluster tells that this is no break, as
/// kubectl (port forward prototype, R2: the prototype broke such an upload after 25 s).
pub(crate) async fn an_upload_of_200_mb_to_a_pod_that_reads_nothing_for_30_s_completes(bed: &Bed) {
    let sink = rule(Kind::Pod, "slow", 9000);
    let address = bed.active(&sink).await;
    let started = Instant::now();
    let mut client = TcpStream::connect(address).await.expect("a connect");
    let piece = vec![7_u8; 1024 * 1024];
    for _ in 0..UPLOAD / piece.len() {
        client.write_all(&piece).await.expect("the upload goes on");
    }
    let written = started.elapsed();
    // The first byte goes to a file of its own: the pause of the pod starts with it.
    let size = (UPLOAD - 1).to_string();
    loop {
        let stat =
            bed.k3s.kubectl(&["--namespace", "forwards", "exec", "slow", "--", "stat", "-c", "%s", "/tmp/upload"]);
        let stat = stat.await;
        if stat.as_ref().is_ok_and(|text| text.trim() == size) {
            break;
        }
        assert!(started.elapsed() < 300 * SECOND, "the pod has the 200 MB in 300 s: {stat:?}");
        sleep(SECOND).await;
    }
    println!("the client wrote 200 MB in {written:?}, and the pod had them after {:?}", started.elapsed());
    let view = bed.core.view();
    let row = forward(&view, &sink).expect("the forward");
    assert_eq!((row.view.state.name(), row.view.connections.reset), ("active", 0), "{row:?}");
    assert!(row.view.bytes.sent >= UPLOAD as u64, "{:?}", row.view.bytes);
    drop(client);
}

/// Plan section 12.3 and the fast soaks, which found the forward "active" with no page for
/// 20 s after a reset storm, while the API server was slow. A reset of each connection each
/// 5 s for 60 s, and then 30 s in which each new connection gets no answer for 12 s: the
/// path stays live, and only the opens of the tunnels can tell. A client asks the page of the
/// Service each second, as the soak does; each run of failed pages shows a state in less
/// than 10 s, and the page comes after the faults.
pub(crate) async fn each_run_of_failed_pages_in_a_reset_storm_and_held_connects_shows_a_state(bed: &Bed) {
    let service = rule(Kind::Service, "svc", 80);
    let address = bed.active(&service).await;
    let fetch =
        || async move { matches!(timeout(8 * SECOND, page(address)).await, Ok(Ok(body)) if body == "Deployment") };
    let active = || forward(&bed.core.view(), &service).is_some_and(|row| row.view.state.name() == "active");
    let faults = async {
        bed.proxy.set(ProxyMode::ResetEach(5 * SECOND));
        sleep(60 * SECOND).await;
        bed.proxy.set(ProxyMode::Pass);
        sleep(10 * SECOND).await;
        bed.proxy.set(ProxyMode::HoldNew { probability: 1.0, min: 12 * SECOND, max: 12 * SECOND });
        sleep(30 * SECOND).await;
        bed.proxy.set(ProxyMode::Pass);
        sleep(20 * SECOND).await;
    };
    let (hidden, failed) = longest_hidden_failure(fetch, active, faults).await;
    println!("{failed} pages failed in the faults; the longest run with no state lasted {hidden:?}");
    assert!(failed >= 3, "the held connects failed pages: {failed}");
    assert!(hidden < 10 * SECOND, "the forward showed \"active\" for {hidden:?} of failed pages");
    until_page(address, "Deployment", 10 * SECOND).await;
}

/// Plan section 15, M5: each forward shows a silent path in 20 s or less, its local port
/// stays open, and it works again 10 s or less after the network returns.
pub(crate) async fn the_local_ports_stay_open_in_a_break_and_work_again_10_s_or_less_after_it(
    bed: &Bed,
    forwards: &[(ForwardRule, SocketAddr)],
) {
    let silent = SystemTime::now();
    bed.proxy.set(ProxyMode::Freeze);
    for (rule, address) in forwards {
        let reconnecting = |view: &fresnel_core::app::CoreView| {
            forward(view, rule).is_some_and(|row| row.view.state.name() == "reconnecting")
        };
        let view = bed.until(&format!("{rule} shows the break"), reconnecting).await;
        let since = forward(&view, rule).map(|row| row.view.since).expect("the forward");
        let after = since.duration_since(silent).unwrap_or_default();
        assert!(after <= 20 * SECOND, "{rule} showed the break after {after:?}");
        assert!(TcpStream::connect(address).await.is_ok(), "the local port of {rule} stays open");
    }
    sleep(10 * SECOND).await;
    let back = Instant::now();
    bed.proxy.set(ProxyMode::Pass);
    each_page(forwards, "the network returned").await;
    assert!(back.elapsed() <= 10 * SECOND, "the forwards worked again after {:?}", back.elapsed());
}

/// How a local client saw its connection end, with no byte of its own.
async fn end_seen(address: SocketAddr) -> Result<&'static str, String> {
    let mut client = TcpStream::connect(address).await.map_err(|error| format!("the port is closed: {error}"))?;
    let mut buffer = [0_u8; 16];
    Ok(match timeout(30 * SECOND, client.read(&mut buffer)).await {
        Ok(Ok(0)) => "end of data",
        Ok(Ok(_)) => "data",
        Ok(Err(_)) => "reset",
        Err(_) => "nothing in 30 s",
    })
}

/// Plan section 15, M5: each forward works again 10 s or less after a wake. In the sleep,
/// each connection went silent.
pub(crate) async fn each_forward_works_again_10_s_or_less_after_a_wake(
    bed: &Bed,
    forwards: &[(ForwardRule, SocketAddr)],
) {
    bed.proxy.silence_open();
    let woke = Instant::now();
    bed.woke.store(true, Ordering::Relaxed);
    each_page(forwards, "the wake").await;
    assert!(woke.elapsed() <= 10 * SECOND, "the forwards worked again after {:?}", woke.elapsed());
}

/// Waits until each forward gives its page, all at the same time, as the clients of many
/// forwards do.
async fn each_page(forwards: &[(ForwardRule, SocketAddr)], after: &str) {
    let pages = forwards.iter().map(|(rule, address)| async move {
        let waited = until_page(*address, &word(rule), 30 * SECOND).await;
        println!("{rule}: the page {waited:?} after {after}");
    });
    futures_util::future::join_all(pages).await;
}

/// The page of the pods of a rule: the kind of the object that owns them.
fn word(rule: &ForwardRule) -> String {
    match rule.kind {
        Kind::Service => "Deployment".to_owned(),
        kind => kind.info().name.to_owned(),
    }
}
