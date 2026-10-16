//! The bytes of a forward: to a Pod and to a Service, to a new pod after the old one went,
//! and to a pod that reads in bursts.

use fresnel_core::app::Health;
use tokio::time::Instant;

use super::*;
use crate::app::row;
use crate::common::in_time;

const FORWARD_PATH: &str = "/portforward";

/// The pod and the port of each port forward request that the server saw.
fn tunnels(server: &FakeApiServer) -> Vec<String> {
    let requests = server.requests().into_iter().filter(|seen| seen.uri.path().ends_with(FORWARD_PATH));
    requests.map(|seen| format!("{}?{}", seen.uri.path(), seen.uri.query().unwrap_or_default())).collect()
}

/// A forward keeps its cluster connected with no selection: a forward reads, and a user who
/// looks at an other cluster keeps the forwards of this one.
#[tokio::test(flavor = "multi_thread")]
async fn a_forward_to_a_pod_carries_its_bytes_and_keeps_its_cluster_connected_with_no_selection() {
    let bed = Bed::start("forward-pod").await;
    bed.app.until_health("ams", Health::Live).await;
    bed.app.select("ams", false);
    bed.app.until_health("ams", Health::Off).await;

    let pod = rule(Kind::Pod, "web-1", 8080);
    bed.start_forward(&pod);
    let address = bed.active(&pod).await;
    let data: Vec<u8> = (0..300_000_u32).map(|byte| byte.to_le_bytes()[0]).collect();
    assert_eq!(echo(address, &data).await, Ok(()));
    let view = bed.app.core.view();
    assert!(!row(&view, "ams").selected && row(&view, "ams").health() == Health::Live, "{:?}", row(&view, "ams"));
    let status = row(&view, "ams").status().cloned().expect("a connected cluster");
    assert_eq!(status.upgrade.holders, 1, "the forward keeps upgrade connections warm");
    let shown = bed.until(&pod, "the counts", |view| view.bytes.received == 300_000).await;
    let target = shown.view.target.as_ref().expect("a target");
    assert_eq!((&*target.pod.name, target.port, target.ready), ("web-1", 8080, true));
    assert_eq!((shown.view.bytes.sent, shown.view.connections.total), (300_000, 1));
    assert_eq!(tunnels(&bed.server), ["/api/v1/namespaces/shop/pods/web-1/portforward?&ports=8080"]);

    // With no forward, the cluster that is not selected disconnects.
    bed.app.core.send(Command::StopForward(shown.id));
    bed.app.until_health("ams", Health::Off).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_forward_to_a_service_reaches_the_target_port_of_a_pod_of_its_endpoint_slices() {
    let bed = Bed::start("forward-service").await;
    let service = rule(Kind::Service, "web", 80);
    bed.start_forward(&service);
    let address = bed.active(&service).await;
    assert_eq!(echo(address, b"GET / HTTP/1.0\r\n\r\n").await, Ok(()));
    assert_eq!(tunnels(&bed.server), ["/api/v1/namespaces/shop/pods/web-1/portforward?&ports=8080"]);
}

/// Plan section 15, M5: a forward recovers 5 s or less after its pod restarts. The pod
/// `web-1` goes, and `web-2` takes its place in the EndpointSlice of the Service.
#[tokio::test(flavor = "multi_thread")]
async fn a_pod_that_goes_gives_way_to_a_new_ready_pod_in_5_s() {
    let bed = Bed::start("forward-new-pod").await;
    let service = rule(Kind::Service, "web", 80);
    bed.start_forward(&service);
    let address = bed.active(&service).await;
    assert_eq!(echo(address, b"one").await, Ok(()));

    let deleted = format!(r#"{{"type":"DELETED","object":{}}}"#, pod("web-1", "u1").replace('\n', ""));
    assert!(bed.server.send_event(PODS, &deleted) > 0, "the watch of the pods");
    let gone = bed.until(&service, "no pod", |view| view.state.name() == "degraded").await;
    assert_eq!(gone.view.state.to_string(), "degraded: the Service shop/web has no pod that runs");

    let new = Instant::now();
    let added = format!(r#"{{"type":"ADDED","object":{}}}"#, pod("web-2", "u2").replace('\n', ""));
    let slice = r#"{"type":"MODIFIED","object":{"metadata":{"name":"web-x1","namespace":"shop","uid":"e1",
        "resourceVersion":"11","labels":{"kubernetes.io/service-name":"web"}},"addressType":"IPv4",
        "endpoints":[{"addresses":["10.0.0.2"],"conditions":{"ready":true},
        "targetRef":{"kind":"Pod","name":"web-2","namespace":"shop","uid":"u2"}}],
        "ports":[{"name":"http","port":8080,"protocol":"TCP"}]}}"#;
    bed.server.send_event(PODS, &added);
    bed.server.send_event(SLICES, &slice.replace('\n', ""));
    in_time("a tunnel to the new pod", async {
        while echo(address, b"two").await.is_err() {
            tokio::task::yield_now().await;
        }
    })
    .await;
    println!("the forward carried bytes to the new pod {:?} after its events", new.elapsed());
    assert!(new.elapsed() <= 5 * SECOND, "the forward recovered after {:?}", new.elapsed());
    let tunnels = tunnels(&bed.server);
    assert_eq!(tunnels.last().map(String::as_str), Some("/api/v1/namespaces/shop/pods/web-2/portforward?&ports=8080"));
}

/// Plan section 15, M5: an upload to a slow pod completes. The pod reads in bursts with pauses
/// of 3 s, longer than the limit of a ping, and a write that waits for it is no break.
#[tokio::test(flavor = "multi_thread")]
async fn an_upload_to_a_pod_that_reads_in_bursts_completes_through_the_forward() {
    let bed = Bed::start("forward-upload").await;
    let pod = rule(Kind::Pod, "web-1", 8080);
    bed.start_forward(&pod);
    let address = bed.active(&pod).await;
    let sent: Vec<u8> = (0..32 * 1024 * 1024_u32).map(|byte| byte.to_le_bytes()[2]).collect();
    let mut client = TcpStream::connect(address).await.expect("a connect");
    let (mut reader, mut writer) = client.split();
    let write = async { writer.write_all(&sent).await.expect("the upload") };
    let read = async {
        let mut back = vec![0_u8; sent.len()];
        reader.read_exact(&mut back).await.expect("the bytes back");
        back
    };
    let slow_pod = async {
        loop {
            bed.server.hold_streams(true);
            tokio::time::sleep(3 * SECOND).await;
            bed.server.hold_streams(false);
            tokio::time::sleep(SECOND / 20).await;
        }
    };
    let until_read = async {
        tokio::select! {
            back = read => back,
            () = slow_pod => unreachable!("the pod pauses until the end of the read"),
        }
    };
    let ((), back) = tokio::join!(write, until_read);
    bed.server.hold_streams(false);
    assert!(back == sent, "each byte came back in its sequence");
    let view = bed.until(&pod, "the counts", |view| view.bytes.received == sent.len() as u64).await;
    assert_eq!((view.view.state.name(), view.view.connections.reset), ("active", 0));
}

/// The protocol has no half close: after the end of the local input, the tunnel stays while
/// the pod sends, and closes after the pod is silent for the linger time. The linger time
/// starts at the end of the input. A client that was quiet longer than that time, then sends
/// its request and ends its input, gets the answer.
#[tokio::test(flavor = "multi_thread")]
async fn a_client_that_ends_its_input_after_a_quiet_time_gets_the_answer() {
    let bed = Bed::start("forward-half-close").await;
    let pod = rule(Kind::Pod, "web-1", 8080);
    bed.start_forward(&pod);
    let address = bed.active(&pod).await;
    let mut client = TcpStream::connect(address).await.expect("a connect");
    in_time("the tunnel of the client", async {
        while tunnels(&bed.server).is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await;
    // The client is quiet for twice the linger time, as a user who types a request.
    tokio::time::sleep(2 * SECOND).await;
    client.write_all(b"the request").await.expect("the request");
    client.shutdown().await.expect("the end of the input");
    let mut back = Vec::new();
    let read = timeout(10 * SECOND, client.read_to_end(&mut back)).await;
    assert!(matches!(read, Ok(Ok(_))), "{read:?}");
    assert_eq!(String::from_utf8_lossy(&back), "the request", "the answer of the pod");
}
