use serde_json::json;

use super::*;

fn list(items: Value) -> RawList {
    serde_json::from_value(json!({ "kind": "PodMetricsList", "items": items })).expect("a list")
}

fn amount(cpu: u64, memory: u64) -> Amount {
    Amount { cpu, memory }
}

/// An item as metrics-server 0.8 writes it, with its labels.
fn pod_item(name: &str, containers: Value) -> Value {
    json!({
        "metadata": { "name": name, "namespace": "shop", "labels": { "app": "web" }, "creationTimestamp": "2026-10-15T10:00:20Z" },
        "timestamp": "2026-10-15T10:00:05Z",
        "window": "15.012s",
        "containers": containers,
    })
}

#[test]
fn a_list_of_pods_gives_the_usage_of_each_container_at_the_time_of_the_server() {
    let containers = json!([
        { "name": "app", "usage": { "cpu": "6082165n", "memory": "22272Ki" } },
        { "name": "proxy", "usage": { "cpu": "1m", "memory": "8Mi" } },
    ]);
    let read = pods(list(json!([pod_item("web-1", containers)])));
    let at = 1_792_058_405;
    let sample = PodSample {
        key: ObjectKey::new("shop", "web-1"),
        at,
        window: Some(Duration::from_millis(15_012)),
        containers: vec![("app".into(), amount(6_082_165, 22_806_528)), ("proxy".into(), amount(1_000_000, 8_388_608))],
    };
    assert_eq!(read, Read { items: vec![sample.clone()], unread: 0, problem: None });
    assert_eq!(sample.usage(), amount(7_082_165, 31_195_136));
}

#[test]
fn a_list_of_nodes_gives_the_usage_of_each_node() {
    let item = json!({
        "metadata": { "name": "node-a" },
        "timestamp": "2026-10-15T10:00:05Z",
        "window": "20.05s",
        "usage": { "cpu": "250m", "memory": "1.5Gi" },
    });
    let node = NodeSample {
        name: "node-a".into(),
        at: 1_792_058_405,
        window: Some(Duration::from_millis(20_050)),
        usage: amount(250_000_000, 1_610_612_736),
    };
    assert_eq!(nodes(list(json!([item]))), Read { items: vec![node], unread: 0, problem: None });
}

/// `k8s-metrics` fails the whole list for one of these items (research 11 section 7.2).
#[test]
fn an_item_that_does_not_fit_is_counted_with_its_reason_and_the_other_items_stay() {
    let good = |name| pod_item(name, json!([{ "name": "app", "usage": { "cpu": "1m", "memory": "1Mi" } }]));
    let no_memory = pod_item("no-memory", json!([{ "name": "app", "usage": { "cpu": "1m" } }]));
    let mut no_time = good("no-time");
    no_time["timestamp"] = json!("yesterday");
    let mut no_window = good("no-window");
    no_window["window"] = json!("a while");
    let cases = [
        (no_memory, "the container app of no-memory: no memory"),
        (
            pod_item("bad-cpu", json!([{ "name": "app", "usage": { "cpu": "1x", "memory": "1Mi" } }])),
            "the container app of bad-cpu: the cpu: an unknown suffix",
        ),
        (pod_item("empty", json!([])), "the pod empty has no namespace or no container"),
        (
            pod_item("nameless", json!([{ "usage": { "cpu": "1m", "memory": "1Mi" } }])),
            "a container of the pod nameless has no name",
        ),
        (no_time, "the time of an item is not RFC 3339"),
        (json!({ "metadata": { "namespace": "shop" } }), "an item has no name"),
        (json!({ "metadata": { "name": 7 } }), "an item does not fit the API"),
        (json!("a text"), "an item does not fit the API"),
    ];
    for (item, problem) in cases {
        let read = pods(list(json!([good("good-1"), item, good("good-2")])));
        assert_eq!((read.items.len(), read.unread, read.problem.as_deref()), (2, 1, Some(problem)));
    }
    // A window is only a hint of the resolution: a bad one costs the sample nothing.
    let read = pods(list(json!([no_window])));
    assert_eq!((read.items.len(), read.items[0].window, read.unread), (1, None, 0));
    let two = pods(list(json!([pod_item("a", json!([])), pod_item("b", json!([]))])));
    assert_eq!((two.unread, two.problem.as_deref()), (2, Some("the pod a has no namespace or no container")));
}

/// Bad data of a server: a second item of the same object would count twice in each sum.
#[test]
fn a_second_item_of_the_same_object_is_unread_and_the_first_stays() {
    let web = |cpu: &str| pod_item("web", json!([{ "name": "app", "usage": { "cpu": cpu, "memory": "1Mi" } }]));
    let read = pods(list(json!([web("1m"), pod_item("other", json!([])), web("2m")])));
    let first = read.items.iter().map(|pod| pod.usage().cpu).collect::<Vec<_>>();
    assert_eq!((first, read.unread), (vec![1_000_000], 2));
    assert_eq!(read.problem.as_deref(), Some("the pod other has no namespace or no container"), "the first reason");
    let node = json!({ "metadata": { "name": "node-a" }, "timestamp": "2026-10-15T10:00:05Z",
        "usage": { "cpu": "1", "memory": "1" } });
    let read = nodes(list(json!([node.clone(), node])));
    assert_eq!((read.items.len(), read.unread, read.problem.as_deref()), (1, 1, Some("the node node-a comes twice")));
}

#[test]
fn a_list_with_no_items_is_an_empty_list_and_a_time_before_1970_is_unread() {
    let empty: RawList = serde_json::from_value(json!({ "kind": "NodeMetricsList" })).expect("a list");
    assert_eq!(nodes(empty), Read { items: vec![], unread: 0, problem: None });
    let old = json!({ "metadata": { "name": "n" }, "timestamp": "1969-12-31T23:59:59Z", "usage": { "cpu": "1", "memory": "1" } });
    let read = nodes(list(json!([old])));
    assert_eq!(read.problem.as_deref(), Some("the time of an item is before 1970 or after 2105"));
}
