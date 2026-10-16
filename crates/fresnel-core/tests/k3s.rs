//! The gateway on a real kube-apiserver: k3s in Docker behind the fault proxy.
//!
//! The steps are in one test, because the start of a cluster is the slow part.

#![allow(clippy::expect_used, reason = "a test stops with a panic")]

#[path = "common/mod.rs"]
mod common;

use std::time::{Duration, Instant};

use fresnel_core::cluster::{Fate, Gateway, WatchEvent};
use fresnel_core::net::{ErrorClass, NetSettings, SocketSpec, SocketState};
use fresnel_testkit::{FaultProxy, K3s};
use k8s_openapi::api::core::v1::{Namespace, Pod};
use k8s_openapi::apimachinery::pkg::version::Info;
use kube::core::params::{DeleteParams, GetParams, ListParams, Patch, PatchParams, PostParams, WatchParams};
use kube::core::subresource::{AttachParams, LogParams};
use kube::core::{ObjectList, Request};
use serde_json::{Value, json};
use tokio::runtime::Handle;
use tokio::time::timeout;

use common::{TestTasks, connect, in_time, registry, until_live};

const NAME: &str = "fresnel-made";
/// The images of the pods of k3s come from the network at the first start.
const POD_LIMIT: Duration = Duration::from_secs(300);

fn namespaces() -> Request {
    Request::new("/api/v1/namespaces")
}

fn system_pods() -> Request {
    Request::new("/api/v1/namespaces/kube-system/pods")
}

fn body(object: &Value) -> Vec<u8> {
    serde_json::to_vec(object).expect("JSON")
}

/// Reads each page of the namespaces, two at a time. Gives the names and the resource version.
async fn all_namespaces(gateway: &Gateway) -> (Vec<String>, String) {
    let (mut names, mut pages) = (Vec::new(), 0);
    let mut params = ListParams::default().limit(2);
    loop {
        let page: ObjectList<Namespace> =
            gateway.list_page(namespaces().list(&params).expect("a request")).await.expect("a page");
        assert!(page.items.len() <= 2, "the page has the size of its limit");
        names.extend(page.items.into_iter().filter_map(|namespace| namespace.metadata.name));
        pages += 1;
        match page.metadata.continue_.filter(|token| !token.is_empty()) {
            Some(token) => params = params.continue_token(&token),
            None => {
                assert!(pages >= 2, "a cluster has four namespaces at its start");
                return (names, page.metadata.resource_version.expect("a resource version"));
            }
        }
    }
}

/// The name of the DNS pod of k3s, when its container is ready. The watch from version 0
/// gives the pod that exists and each change after it.
async fn ready_dns_pod(gateway: &Gateway) -> String {
    let params = WatchParams::default().labels("k8s-app=kube-dns").timeout(290);
    let mut events =
        gateway.watch_stream::<Pod>(system_pods().watch(&params, "0").expect("a request")).await.expect("a watch");
    let ready = async {
        loop {
            let event = events.next().await.expect("the watch has no end before its timeout").expect("an event");
            let (WatchEvent::Added(pod) | WatchEvent::Modified(pod)) = event else { continue };
            let conditions = pod.status.and_then(|status| status.conditions).unwrap_or_default();
            if conditions.iter().any(|condition| condition.type_ == "Ready" && condition.status == "True") {
                return pod.metadata.name.expect("a name");
            }
        }
    };
    timeout(POD_LIMIT, ready).await.expect("the DNS pod of k3s is ready in time")
}

#[tokio::test]
#[ignore = "needs k3s in Docker"]
async fn the_six_functions_work_on_a_real_kube_apiserver_through_the_fault_proxy() {
    let k3s = K3s::start("core").await.expect("k3s starts");
    let proxy = FaultProxy::start(k3s.api_addr()).await.expect("a proxy");
    let kubeconfig = k3s.kubeconfig_through(proxy.addr()).expect("a kubeconfig");
    let tasks = TestTasks::new(&Handle::current());
    let registry = registry(&tasks, &Handle::current(), &kubeconfig, NetSettings::default());
    let link = connect(&registry, "default");
    let gateway = &link.gateway;

    // get: a small read, an object, and an object that does not exist.
    let version: Info =
        gateway.get(http::Request::get("/version").body(Vec::new()).expect("a request")).await.expect("the version");
    assert_eq!(version.git_version, "v1.33.5+k3s1");
    let get = |name: &str| namespaces().get(name, &GetParams::default()).expect("a request");
    let system: Namespace = gateway.get(get("kube-system")).await.expect("a namespace");
    assert_eq!(system.status.and_then(|status| status.phase).as_deref(), Some("Active"));
    let error = gateway.get::<Namespace>(get(NAME)).await.expect_err("no such namespace");
    assert_eq!((error.class(), error.status()), (ErrorClass::Invalid, Some(404)));
    assert_eq!(error.to_string(), format!("namespaces \"{NAME}\" not found (HTTP 404)"));

    // list_page, then watch_stream from the version of the list.
    let (names, version) = all_namespaces(gateway).await;
    assert!(["default", "kube-system"].iter().all(|name| names.iter().any(|listed| listed == name)), "{names:?}");
    let watch = namespaces().watch(&WatchParams::default().timeout(120), &version).expect("a request");
    let mut events = gateway.watch_stream::<Namespace>(watch).await.expect("a watch");

    // send: create, a second create, patch, replace with an old version, delete.
    let create = || namespaces().create(&PostParams::default(), body(&json!({ "metadata": { "name": NAME } })));
    let made: Namespace = gateway.send(create().expect("a request")).await.expect("the namespace");
    let old_version = made.metadata.resource_version.clone().expect("a resource version");
    let error = gateway.send::<Namespace>(create().expect("a request")).await.expect_err("the name is in use");
    assert_eq!(
        (error.fate, error.error.class(), error.error.status()),
        (Fate::Refused, ErrorClass::Conflict, Some(409))
    );

    let label = Patch::Merge(json!({ "metadata": { "labels": { "fresnel": "yes" } } }));
    let patch = namespaces().patch(NAME, &PatchParams::default(), &label).expect("a request");
    let patched: Namespace = gateway.send(patch).await.expect("the patched namespace");
    assert_eq!(patched.metadata.labels.and_then(|labels| labels.get("fresnel").cloned()).as_deref(), Some("yes"));
    let in_cluster = k3s.kubectl(&["get", "namespace", NAME, "--output", "jsonpath={.metadata.labels.fresnel}"]).await;
    assert_eq!(in_cluster.expect("kubectl works"), "yes", "the state of the cluster, as kubectl reads it");

    let stale = json!({ "metadata": { "name": NAME, "resourceVersion": old_version } });
    let replace = namespaces().replace(NAME, &PostParams::default(), body(&stale)).expect("a request");
    let error = gateway.send::<Namespace>(replace).await.expect_err("the object changed after that version");
    assert_eq!(
        (error.fate, error.error.class(), error.error.status()),
        (Fate::Refused, ErrorClass::Conflict, Some(409))
    );

    for expected in ["added", "modified"] {
        let event = in_time("an event of the watch", events.next()).await.expect("an event").expect("a good event");
        let (kind, namespace) = match event {
            WatchEvent::Added(namespace) => ("added", namespace),
            WatchEvent::Modified(namespace) => ("modified", namespace),
            other => panic!("{other:?}"),
        };
        assert_eq!((kind, namespace.metadata.name.as_deref()), (expected, Some(NAME)));
    }
    let delete = namespaces().delete(NAME, &DeleteParams::default()).expect("a request");
    let deleted: Value = gateway.send(delete).await.expect("the answer to the delete");
    assert_eq!(deleted["status"]["phase"], "Terminating", "{deleted}");

    // A reset of each connection: the watch ends with an error, and the next read works
    // on the new connection of its channel.
    in_time("the reset", proxy.reset_all()).await;
    let reset_at = Instant::now();
    let ended = in_time("the end of the watch", async {
        loop {
            match events.next().await {
                Some(Ok(_)) => {}
                Some(Err(error)) => return Some(error.class()),
                None => return None,
            }
        }
    });
    assert_eq!(ended.await, Some(ErrorClass::Network), "the watch of a reset connection ends with an error");
    let (names, _) = all_namespaces(gateway).await;
    assert!(names.iter().any(|name| name == "kube-system"), "{names:?}");
    assert!(reset_at.elapsed() < Duration::from_secs(5), "the recovery took {:?}", reset_at.elapsed());
    until_live(&link).await;

    // log_stream and upgrade need a container: the DNS pod of k3s.
    let pod = ready_dns_pod(gateway).await;
    let follow = LogParams { follow: true, ..LogParams::default() };
    let mut log = gateway.log_stream(system_pods().logs(&pod, &follow).expect("a request")).await.expect("a log");
    let mut text = String::new();
    while !text.contains("CoreDNS-") {
        let piece = in_time("the log of the DNS server", log.next()).await.expect("the log follows").expect("data");
        text.push_str(&String::from_utf8_lossy(&piece));
    }
    drop(log);

    let exec = system_pods().exec(&pod, ["/coredns", "-version"], &AttachParams::default()).expect("a request");
    let spec = SocketSpec { role: "exec socket", protocols: &["v5.channel.k8s.io", "v4.channel.k8s.io"], on_end: None };
    let mut socket = gateway.upgrade(exec, spec).await.expect("the WebSocket of the exec");
    assert_eq!(socket.protocol(), "v5.channel.k8s.io");
    let mut output = Vec::new();
    // The server ends the connection after the end of the program, with or with no close message.
    while let Some(data) = in_time("the output of the program", socket.recv()).await {
        // The first byte is the channel: 1 is the standard output.
        if data.first() == Some(&1) {
            output.extend_from_slice(&data[1..]);
        }
    }
    let output = String::from_utf8_lossy(&output);
    assert!(output.starts_with("CoreDNS-"), "the output of `/coredns -version`: {output}");
    match socket.state() {
        SocketState::Ended(end) => assert!(end.is_clean(), "{end}"),
        other => panic!("the socket ended: {other:?}"),
    }
    assert_eq!(link.net.status().upgrade.opened, 1, "the exec used one upgrade connection");
    assert_eq!(proxy.task_failures(), Vec::<String>::new());
}
