//! The six functions of the gateway on the fake API server.

use std::time::{Duration, Instant};

use fresnel_core::CoreError;
use fresnel_core::cluster::{Fate, WatchEvent};
use fresnel_core::net::{ErrorClass, Limits, NetSettings, SocketSpec, SocketState};
use fresnel_testkit::{FakeApiServer, Fault, FaultProxy};
use http::header::{SEC_WEBSOCKET_PROTOCOL, UPGRADE};
use http::{Method, Version};
use k8s_openapi::api::core::v1::{ConfigMap, Namespace};
use k8s_openapi::apimachinery::pkg::version::Info;
use kube::core::ObjectList;
use tokio::runtime::Handle;

use crate::bed::{Bed, Folder, get, kubeconfig, post};
use crate::common::{TestTasks, connect, in_time, registry, until_live};

const MS: Duration = Duration::from_millis(1);
const NAMESPACES: &str = "/api/v1/namespaces";
const CONFIG_MAPS: &str = "/api/v1/namespaces/default/configmaps";
const LOG: &str = "/api/v1/namespaces/default/pods/web/log";
const EXEC: &str = "/api/v1/namespaces/default/pods/web/exec?command=sh&stdout=true";
const TWO_NAMESPACES: &str = r#"{"kind":"NamespaceList","apiVersion":"v1",
    "metadata":{"resourceVersion":"7","continue":"next"},
    "items":[{"metadata":{"name":"default"}},{"metadata":{"name":"kube-system"}}]}"#;

/// A socket of the test, with the subprotocols of an exec.
fn spec() -> SocketSpec {
    SocketSpec { role: "test socket", protocols: &["v5.channel.k8s.io", "v4.channel.k8s.io"], on_end: None }
}

fn paths(server: &FakeApiServer, method: &Method) -> Vec<String> {
    let of_method = server.requests().into_iter().filter(|seen| seen.method == *method);
    let paths = of_method.filter_map(|seen| seen.uri.path_and_query().map(|path| path.as_str().to_owned()));
    // The light requests of the channels are not requests of a test.
    paths.filter(|path| path != "/version").collect()
}

#[tokio::test]
async fn each_read_function_gives_the_answer_of_the_server_as_its_type() {
    let bed = Bed::start(NetSettings::default()).await;
    let gateway = &bed.link.gateway;
    assert_eq!(gateway.cluster().as_str(), "bed");

    let version: Info = gateway.get(get("/version")).await.expect("the version");
    assert_eq!((version.git_version.as_str(), version.platform.as_str()), ("v1.33.5+fake", "linux/arm64"));

    bed.server.set_body(NAMESPACES, TWO_NAMESPACES);
    let page: ObjectList<Namespace> = gateway.list_page(get("/api/v1/namespaces?limit=2")).await.expect("a page");
    let names: Vec<_> = page.items.iter().filter_map(|namespace| namespace.metadata.name.as_deref()).collect();
    assert_eq!((names, page.metadata.continue_.as_deref()), (vec!["default", "kube-system"], Some("next")));

    let watch = get("/api/v1/namespaces?watch=true&resourceVersion=7");
    let mut events = gateway.watch_stream::<Namespace>(watch).await.expect("a watch");
    assert_eq!(bed.server.send_event(NAMESPACES, r#"{"type":"ADDED","object":{"metadata":{"name":"made"}}}"#), 1);
    let event = in_time("the event", events.next()).await.expect("an event").expect("a good event");
    let WatchEvent::Added(namespace) = event else { panic!("{event:?}") };
    assert_eq!(namespace.metadata.name.as_deref(), Some("made"));
    assert_eq!(bed.server.end_watches(), 1);
    assert!(in_time("the end", events.next()).await.is_none(), "the server timeout ends the stream with no error");

    bed.server.set_body(LOG, "line one\nline two\n");
    let mut log = gateway.log_stream(get(LOG)).await.expect("a log");
    let mut text = Vec::new();
    while let Some(piece) = in_time("the log", log.next()).await {
        text.extend_from_slice(&piece.expect("data"));
    }
    assert_eq!(text, b"line one\nline two\n");

    bed.server.set_body(CONFIG_MAPS, r#"{"metadata":{"name":"made","resourceVersion":"8"}}"#);
    let created: ConfigMap = gateway.send(post(CONFIG_MAPS, r#"{"metadata":{"name":"made"}}"#)).await.expect("a write");
    assert_eq!(created.metadata.resource_version.as_deref(), Some("8"));
    assert_eq!(paths(&bed.server, &Method::POST), [CONFIG_MAPS]);
    assert_eq!(bed.server.counters().connections, 3, "each request used a channel of the cluster");
}

/// Each kind has its own time limit and its own connections. The limits of this test have
/// five values, so a function with the incorrect kind fails at the incorrect time.
#[tokio::test]
async fn each_function_fails_at_the_time_limit_of_its_kind_on_the_connections_of_its_kind() {
    let limits = Limits {
        get: 1000 * MS,
        stream_start: 1400 * MS,
        upgrade: 1800 * MS,
        list_idle: 2200 * MS,
        list_total: 2200 * MS,
        write: 2600 * MS,
    };
    let bed = Bed::start(NetSettings { limits, ..NetSettings::default() }).await;
    let gateway = &bed.link.gateway;
    bed.server.add_fault(Fault::no_answer().on_path("/api/v1/silent"));
    let path = |name: &str| format!("/api/v1/silent/{name}");
    let started = Instant::now();
    let timed = |result: Result<(), CoreError>| {
        let error = result.expect_err("the server gives no answer");
        assert_eq!(error.class(), ErrorClass::Timeout, "{error}");
        started.elapsed().as_millis() / 200
    };
    let requests = async {
        tokio::join!(
            async { timed(gateway.get::<Info>(get(&path("get"))).await.map(drop)) },
            async { timed(gateway.watch_stream::<Namespace>(get(&path("watch?watch=true"))).await.map(drop)) },
            async { timed(gateway.log_stream(get(&path("log"))).await.map(drop)) },
            async { timed(gateway.upgrade(get(&path("exec")), spec()).await.map(drop)) },
            async { timed(gateway.list_page::<Info>(get(&path("list"))).await.map(drop)) },
            async {
                let error = gateway.send::<Info>(post(&path("write"), "{}")).await.expect_err("no answer");
                assert_eq!(error.fate, Fate::Unknown, "the request went out, so the write can be in effect");
                timed(Err(error.error))
            },
        )
    };
    let lanes = async {
        bed.server.wait_for(|counters| counters.unanswered == 6).await;
        let status = bed.link.net.status();
        let open =
            |group: &fresnel_core::net::GroupStatus| group.channels.iter().map(|c| c.streams_open).sum::<usize>();
        (open(&status.watches), open(&status.logs), open(&status.requests), status.upgrade.opened)
    };
    let (steps, lanes) = tokio::join!(requests, in_time("six requests with no answer", lanes));
    assert_eq!(lanes, (1, 1, 3, 1), "watch, log, three requests and one upgrade connection");
    assert_eq!(steps, (5, 7, 7, 9, 11, 13), "the time of each failure in steps of 200 ms");
    assert_eq!(paths(&bed.server, &Method::POST), [path("write")], "the write went out one time");
}

#[tokio::test]
async fn an_error_status_gives_the_message_of_the_server_with_its_class() {
    let bed = Bed::start(NetSettings::default()).await;
    let gateway = &bed.link.gateway;
    bed.server.add_fault(Fault::unauthorized().on_path("/api/v1/secrets"));
    let refused = |error: CoreError| (error.class(), error.status(), error.to_string());
    let expected = (ErrorClass::Auth, Some(401), "Unauthorized (HTTP 401)".to_owned());

    let error = gateway.get::<Info>(get("/api/v1/secrets/a")).await.expect_err("refused");
    assert_eq!(refused(error), expected);
    let error = gateway.list_page::<Info>(get("/api/v1/secrets")).await.expect_err("refused");
    assert_eq!(refused(error), expected);
    // A watch is live only after a good status. The error is not the first event.
    let error = gateway.watch_stream::<Namespace>(get("/api/v1/secrets?watch=true")).await.expect_err("refused");
    assert_eq!(refused(error), expected);
    let error = gateway.log_stream(get("/api/v1/secrets/a/log")).await.expect_err("refused");
    assert_eq!(refused(error), expected);
    let error = gateway.send::<Info>(post("/api/v1/secrets", "{}")).await.expect_err("refused");
    assert_eq!((error.fate, refused(error.error)), (Fate::Refused, expected));
}

#[tokio::test]
async fn a_throttled_write_is_refused_after_the_repeats_of_the_connection_layer() {
    let bed = Bed::start(NetSettings::default()).await;
    bed.server.add_fault(Fault::too_many_requests(Duration::ZERO).on_path(NAMESPACES));
    let error = bed.link.gateway.send::<Namespace>(post(NAMESPACES, "{}")).await.expect_err("throttled");
    assert_eq!((error.fate, error.error.class(), error.error.status()), (Fate::Refused, ErrorClass::Server, Some(429)));
    assert_eq!(
        error.to_string(),
        "the server refused the write: Too many requests, please try again later. (HTTP 429)"
    );
    assert_eq!(paths(&bed.server, &Method::POST).len(), 4, "HTTP 429 is a definite refusal, so three repeats are safe");
}

#[tokio::test]
async fn a_request_in_the_incorrect_function_does_not_go_out() {
    let bed = Bed::start(NetSettings::default()).await;
    let gateway = &bed.link.gateway;
    let error = gateway.send::<Namespace>(get(NAMESPACES)).await.expect_err("a read is not a write");
    assert_eq!((error.fate, error.error.class()), (Fate::NotSent, ErrorClass::Invalid));
    assert_eq!(error.error.to_string(), "`send` sends a write, and the request has the method GET");

    let write = || post(NAMESPACES, "{}");
    let texts = [
        gateway.get::<Namespace>(write()).await.expect_err("a write").to_string(),
        gateway.list_page::<Namespace>(write()).await.expect_err("a write").to_string(),
        gateway.watch_stream::<Namespace>(write()).await.expect_err("a write").to_string(),
        gateway.log_stream(write()).await.expect_err("a write").to_string(),
        gateway.upgrade(write(), spec()).await.map(drop).expect_err("a write").to_string(),
    ];
    let expected = ["get", "list_page", "watch_stream", "log_stream"]
        .map(|function| format!("`{function}` sends a read, and the request has the method POST"));
    assert_eq!(texts[..4], expected);
    assert_eq!(texts[4], "`upgrade` sends an upgrade, and the request has the method POST");
    assert_eq!(bed.server.requests().iter().filter(|seen| seen.uri.path() != "/version").count(), 0);
}

#[tokio::test]
async fn the_error_of_an_answer_that_does_not_fit_has_no_value_of_the_object() {
    let bed = Bed::start(NetSettings::default()).await;
    bed.server.set_body("/api/v1/namespaces/a/secrets/b", r#""secret-value-1""#);
    let error = bed.link.gateway.get::<Info>(get("/api/v1/namespaces/a/secrets/b")).await.expect_err("not a version");
    let expected = "the answer of the server is not a `k8s_openapi::v1_33::apimachinery::pkg::version::info::Info`: \
                    JSON error at line 1, column 16";
    assert_eq!((error.class(), error.to_string().as_str()), (ErrorClass::Invalid, expected));

    // The server took the write, and its answer is not the expected object.
    let error =
        bed.link.gateway.send::<Info>(post("/api/v1/namespaces/a/secrets/b", "{}")).await.expect_err("no version");
    assert_eq!((error.fate, error.error.to_string().as_str()), (Fate::Unknown, expected));
}

/// A fault at the worst moment for a write: the server took the request and gave a good
/// status, and the connection breaks before the end of the answer. The write does not go
/// out again on the new connection (design rule 6).
#[tokio::test]
async fn a_write_with_a_broken_answer_has_an_unknown_fate_and_goes_out_one_time() {
    let server = FakeApiServer::start().await.expect("a server");
    let proxy = FaultProxy::start(server.addr()).await.expect("a proxy");
    let folder = Folder::new("cut");
    let path = folder.write("config", &kubeconfig(&[("cut", proxy.addr(), &server)]));
    let tasks = TestTasks::new(&Handle::current());
    let registry = registry(&tasks, &Handle::current(), &path, NetSettings::default());
    let link = connect(&registry, "cut");
    until_live(&link).await;
    let gateway = &link.gateway;

    // The fake server answers `watch=true` with a good status and a body that has no end.
    let write = "/api/v1/namespaces?watch=true";
    let cut = async {
        in_time("the server took the write", server.wait_for(|counters| counters.open_watches == 1)).await;
        // This answer comes after the status of the write, on the same connection.
        let _: Info = gateway.get(get("/version")).await.expect("the version");
        in_time("the reset", proxy.reset_all()).await;
    };
    let (result, ()) = tokio::join!(gateway.send::<Namespace>(post(write, "{}")), cut);
    let error = result.expect_err("the answer broke");
    assert_eq!((error.fate, error.error.class()), (Fate::Unknown, ErrorClass::Network), "{error}");
    assert!(error.to_string().starts_with("the result of the write is not known: the answer broke: "), "{error}");

    let _: Info = gateway.get(get("/version")).await.expect("the version, on a new connection");
    until_live(&link).await;
    assert_eq!(paths(&server, &Method::POST), [write], "the write went out one time");
    assert_eq!(server.counters().connections, 6, "one new connection for each channel");
    assert_eq!(proxy.task_failures(), Vec::<String>::new());
}

/// An upgrade goes out on an HTTP/1.1 connection with the headers of a WebSocket, and its
/// answer is a socket with the subprotocol that the server selected.
#[tokio::test]
async fn an_upgrade_goes_out_on_its_own_http1_connection_and_gives_a_socket() {
    let bed = Bed::start(NetSettings::default()).await;
    let socket = bed.link.gateway.upgrade(get(EXEC), spec()).await.expect("a socket");
    assert_eq!((socket.protocol(), socket.state()), ("v5.channel.k8s.io", SocketState::Open));

    let requests = bed.server.requests();
    let seen = requests.iter().find(|seen| seen.uri.path().ends_with("/exec")).expect("the upgrade request");
    assert_eq!((seen.version, seen.uri.path_and_query().map(|path| path.as_str())), (Version::HTTP_11, Some(EXEC)));
    assert_eq!(seen.headers[UPGRADE], "websocket");
    assert_eq!(seen.headers[SEC_WEBSOCKET_PROTOCOL], "v5.channel.k8s.io, v4.channel.k8s.io");
    assert_eq!((bed.server.counters().upgrades, bed.link.net.status().upgrade.opened), (1, 1));
    assert_eq!(bed.link.net.status().upgrade.sockets.open, 1);
}

/// kube-rs drops the body of a refused upgrade, so its error had the status only (review
/// attack A4 of the terminal prototype). The gateway gives the message of the server.
#[tokio::test]
async fn a_refused_upgrade_gives_the_message_of_the_server() {
    let bed = Bed::start(NetSettings::default()).await;
    bed.server.add_fault(Fault::forbidden().on_path("/api/v1/namespaces/default/pods/web/exec"));
    let error = bed.link.gateway.upgrade(get(EXEC), spec()).await.map(drop).expect_err("refused");
    assert_eq!((error.class(), error.status()), (ErrorClass::Forbidden, Some(403)));
    assert_eq!(error.to_string(), "forbidden: the user of the test cannot read this path (HTTP 403)");
    assert_eq!(bed.link.net.status().upgrade.sockets.opened, 0);
}

/// Design rule 8: a request reaches the cluster of its gateway and no other one.
#[tokio::test]
async fn each_gateway_sends_to_its_own_cluster() {
    let (east, west) =
        (FakeApiServer::start().await.expect("a server"), FakeApiServer::start().await.expect("a server"));
    east.set_body(NAMESPACES, r#"{"metadata":{},"items":[{"metadata":{"name":"only-east"}}]}"#);
    west.set_body(NAMESPACES, r#"{"metadata":{},"items":[{"metadata":{"name":"only-west"}}]}"#);
    let folder = Folder::new("two");
    let path = folder.write("config", &kubeconfig(&[("east", east.addr(), &east), ("west", west.addr(), &west)]));
    let tasks = TestTasks::new(&Handle::current());
    let registry = registry(&tasks, &Handle::current(), &path, NetSettings::default());
    let links = [connect(&registry, "east"), connect(&registry, "west")];

    for (link, name) in links.iter().zip(["east", "west"]) {
        assert_eq!(link.gateway.cluster().as_str(), name);
        let page: ObjectList<Namespace> = link.gateway.list_page(get(NAMESPACES)).await.expect("a page");
        assert_eq!(page.items[0].metadata.name, Some(format!("only-{name}")));
        let made: Namespace = link.gateway.send(post(NAMESPACES, "{}")).await.expect("a write");
        assert_eq!(made.metadata.name, None);
    }
    for server in [&east, &west] {
        assert_eq!(paths(server, &Method::POST).len(), 1, "one write for each cluster");
        assert_eq!(paths(server, &Method::GET), [NAMESPACES]);
    }
}
