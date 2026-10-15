use serde_json::{Value, json};

use super::*;

const MI: u64 = 1024 * 1024;
const MILLI: u64 = 1_000_000;

fn pod(spec: Value) -> Pod {
    serde_json::from_value(json!({ "metadata": { "name": "web" }, "spec": spec })).expect("a pod")
}

/// A container with these requests and limits of CPU in millicores and memory in MiB.
fn container(name: &str, requests: (u64, u64), limits: Option<(u64, u64)>) -> Value {
    let values = |(cpu, memory): (u64, u64)| json!({ "cpu": format!("{cpu}m"), "memory": format!("{memory}Mi") });
    let mut resources = json!({ "requests": values(requests) });
    if let Some(limits) = limits {
        resources["limits"] = values(limits);
    }
    json!({ "name": name, "resources": resources })
}

fn sidecar(name: &str, requests: (u64, u64), limits: (u64, u64)) -> Value {
    let mut container = container(name, requests, Some(limits));
    container["restartPolicy"] = json!("Always");
    container
}

fn amount(millicores: u64, mebibytes: u64) -> Amount {
    Amount { cpu: millicores * MILLI, memory: mebibytes * MI }
}

fn limits(millicores: u64, mebibytes: u64) -> Limits {
    Limits { cpu: Some(millicores * MILLI), memory: Some(mebibytes * MI) }
}

#[test]
fn the_requests_and_limits_of_a_pod_are_the_sums_of_its_containers() {
    let spec = json!({ "containers": [
        container("app", (250, 128), Some((500, 256))),
        container("proxy", (50, 32), Some((100, 64))),
    ] });
    assert_eq!(of_pod(&pod(spec)), PodResources { requests: amount(300, 160), limits: limits(600, 320) });
}

#[test]
fn an_init_container_counts_when_it_needs_more_than_the_containers() {
    let spec = json!({
        "initContainers": [container("migrate", (1000, 64), Some((2000, 64)))],
        "containers": [container("app", (250, 128), Some((500, 256)))],
    });
    assert_eq!(of_pod(&pod(spec)), PodResources { requests: amount(1000, 128), limits: limits(2000, 256) });
}

/// A sidecar runs next to the containers and to each later init container.
#[test]
fn a_sidecar_adds_to_the_containers_and_to_each_later_init_container() {
    let spec = json!({
        "initContainers": [
            sidecar("log", (100, 16), (200, 32)),
            container("migrate", (900, 512), Some((1000, 512))),
        ],
        "containers": [container("app", (250, 128), Some((500, 256)))],
    });
    // The containers: 250 + 100 and 128 + 16. The init step: 900 + 100 and 512 + 16.
    assert_eq!(of_pod(&pod(spec)), PodResources { requests: amount(1000, 528), limits: limits(1200, 544) });
}

#[test]
fn a_container_with_no_limit_gives_the_pod_no_limit_of_that_resource() {
    let mut app = container("app", (250, 128), Some((500, 256)));
    app["resources"]["limits"] = json!({ "memory": "256Mi" });
    let spec = json!({ "containers": [app, container("proxy", (50, 32), Some((100, 64)))] });
    let limits = Limits { cpu: None, memory: Some(320 * MI) };
    assert_eq!(of_pod(&pod(spec)), PodResources { requests: amount(300, 160), limits });

    // An init container with no limit runs with no limit of the pod, as in the kubelet.
    let spec = json!({
        "initContainers": [container("migrate", (100, 64), None)],
        "containers": [container("app", (250, 128), Some((500, 256)))],
    });
    assert_eq!(of_pod(&pod(spec)).limits, Limits::default());
}

#[test]
fn a_value_of_the_pod_level_wins_and_the_overhead_adds_to_each_value() {
    let spec = json!({
        "resources": { "requests": { "cpu": "2" }, "limits": { "cpu": "4" } },
        "overhead": { "cpu": "10m", "memory": "20Mi" },
        "containers": [container("app", (250, 128), Some((500, 256)))],
    });
    let resources = of_pod(&pod(spec));
    assert_eq!(resources.requests, amount(2010, 148));
    assert_eq!(resources.limits, limits(4010, 276));
}

#[test]
fn a_pod_with_no_spec_or_no_values_requests_nothing_and_has_no_limit() {
    let bare: Pod = serde_json::from_value(json!({ "metadata": { "name": "web" } })).expect("a pod");
    assert_eq!(of_pod(&bare), PodResources::default());
    let empty = pod(json!({ "containers": [{ "name": "app" }] }));
    assert_eq!(of_pod(&empty), PodResources { requests: Amount::default(), limits: Limits::default() });
}

#[test]
fn only_a_pod_that_succeeded_or_failed_is_done() {
    let in_phase = |phase: Option<&str>| {
        let status = phase.map_or(json!({}), |phase| json!({ "phase": phase }));
        let pod: Pod =
            serde_json::from_value(json!({ "metadata": { "name": "web" }, "status": status })).expect("a pod");
        is_done(&pod)
    };
    let phases = [Some("Pending"), Some("Running"), Some("Succeeded"), Some("Failed"), Some("Unknown"), None];
    assert_eq!(phases.map(in_phase), [false, false, true, true, false, false]);
}

#[test]
fn a_sum_of_amounts_stops_at_the_largest_value() {
    let large = Amount { cpu: u64::MAX - 1, memory: 1 };
    assert_eq!([large, large].into_iter().sum::<Amount>(), Amount { cpu: u64::MAX, memory: 2 });
}
