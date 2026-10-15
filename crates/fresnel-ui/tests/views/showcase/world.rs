//! The world of the showcase: one shop with three clusters, each name made up.
//!
//! prod-eu and staging are live. prod-us lost its connection 48 s ago, so each of its stores
//! keeps its objects as old ones. A drain of the node pool-general-7x2kw of prod-eu runs: it
//! evicted the pods of cart, grafana and coredns there, whose new pods run on the other
//! nodes or wait for one, and a disruption budget holds postgres-1. On staging, a new
//! version of checkout-api crashes.

use std::sync::Arc;

use k8s_openapi::api::core::v1::Pod;

use super::objects::{App, DAY, HOUR, MINUTE, Owner, PodPlan, State, object, pod, suffix, uid};

const CHECKOUT: App = App {
    namespace: "shop",
    name: "checkout-api",
    owner: Owner::Deployment("7d9c5b8f6"),
    image: "shop/checkout-api:1.42.3",
    requests: ["200m", "256Mi"],
    limits: [Some("500m"), Some("512Mi")],
    usage: [190.0, 300.0],
};
/// The version on staging that crashes.
const CHECKOUT_NEXT: App =
    App { owner: Owner::Deployment("59f7c4d8b"), image: "shop/checkout-api:1.43.0-rc.1", ..CHECKOUT };
const CART: App = App {
    namespace: "shop",
    name: "cart",
    owner: Owner::Deployment("5c8f7b9d6"),
    image: "shop/cart:2.8.1",
    requests: ["100m", "128Mi"],
    limits: [Some("500m"), Some("256Mi")],
    usage: [70.0, 150.0],
};
const REDIS: App = App {
    namespace: "shop",
    name: "redis",
    owner: Owner::StatefulSet,
    image: "redis:7.4.1",
    requests: ["100m", "256Mi"],
    limits: [None, Some("512Mi")],
    usage: [24.0, 190.0],
};
const PAYMENTS: App = App {
    namespace: "payments",
    name: "payments-worker",
    owner: Owner::Deployment("6b4d8c7f9"),
    image: "payments/worker:3.15.0",
    requests: ["200m", "256Mi"],
    limits: [Some("1"), Some("512Mi")],
    usage: [120.0, 230.0],
};
const POSTGRES: App = App {
    namespace: "payments",
    name: "postgres",
    owner: Owner::StatefulSet,
    image: "postgres:17.2",
    requests: ["500m", "1Gi"],
    limits: [Some("2"), Some("2Gi")],
    usage: [340.0, 1330.0],
};
const REPORT: App = App {
    namespace: "payments",
    name: "nightly-report",
    owner: Owner::CronJob,
    image: "payments/report:3.15.0",
    requests: ["200m", "256Mi"],
    limits: [None, Some("1Gi")],
    usage: [0.0, 0.0],
};
const INDEXER: App = App {
    namespace: "search",
    name: "search-indexer",
    owner: Owner::Deployment("84f6d9b5c"),
    image: "search/indexer:0.19.2",
    requests: ["500m", "1Gi"],
    limits: [Some("2"), Some("3Gi")],
    usage: [780.0, 1900.0],
};
const PROMETHEUS: App = App {
    namespace: "monitoring",
    name: "prometheus",
    owner: Owner::StatefulSet,
    image: "prom/prometheus:v3.1.0",
    requests: ["300m", "1Gi"],
    limits: [None, Some("2Gi")],
    usage: [210.0, 1400.0],
};
const GRAFANA: App = App {
    namespace: "monitoring",
    name: "grafana",
    owner: Owner::Deployment("7f5b6c8d4"),
    image: "grafana/grafana:11.4.0",
    requests: ["100m", "128Mi"],
    limits: [None, Some("256Mi")],
    usage: [15.0, 110.0],
};
const NODE_EXPORTER: App = App {
    namespace: "monitoring",
    name: "node-exporter",
    owner: Owner::DaemonSet,
    image: "prom/node-exporter:v1.8.2",
    requests: ["50m", "32Mi"],
    limits: [None, Some("64Mi")],
    usage: [8.0, 18.0],
};
const COREDNS: App = App {
    namespace: "kube-system",
    name: "coredns",
    owner: Owner::Deployment("5d78c9869d"),
    image: "coredns/coredns:1.11.4",
    requests: ["100m", "70Mi"],
    limits: [None, Some("170Mi")],
    usage: [6.0, 28.0],
};
const METRICS_SERVER: App = App {
    namespace: "kube-system",
    name: "metrics-server",
    owner: Owner::Deployment("6f8c7b9d5"),
    image: "registry.k8s.io/metrics-server/metrics-server:v0.7.2",
    requests: ["100m", "200Mi"],
    limits: [None, None],
    usage: [9.0, 34.0],
};

/// The namespaces of each cluster.
pub(super) const NAMESPACES: [&str; 6] = ["default", "kube-system", "monitoring", "payments", "search", "shop"];

/// One cluster: its name, the version of its server, its nodes, and its apps with the
/// count of their pods and the age of their last rollout.
pub(super) struct Site {
    pub(super) name: &'static str,
    pub(super) version: &'static str,
    pub(super) nodes: &'static [&'static str],
    pub(super) apps: &'static [(&'static App, u32, u64)],
    /// False for a cluster that lost its connection: its stores keep old objects.
    pub(super) live: bool,
}

pub(super) const PROD_EU: Site = Site {
    name: "prod-eu",
    version: "v1.33.5",
    nodes: &["pool-general-7x2kp", "pool-general-7x2kr", DRAINED],
    apps: &[
        (&CHECKOUT, 3, 2 * DAY + 4 * HOUR),
        (&CART, 3, 6 * DAY + HOUR),
        (&REDIS, 1, 34 * DAY),
        (&PAYMENTS, 2, 2 * DAY + 5 * HOUR),
        (&POSTGRES, 2, 61 * DAY),
        (&REPORT, 1, 6 * HOUR),
        (&INDEXER, 1, 9 * HOUR + 20 * MINUTE),
        (&PROMETHEUS, 1, 17 * DAY),
        (&GRAFANA, 1, 17 * DAY),
        (&NODE_EXPORTER, 3, 17 * DAY),
        (&COREDNS, 2, 61 * DAY),
        (&METRICS_SERVER, 1, 61 * DAY),
    ],
    live: true,
};

pub(super) const PROD_US: Site = Site {
    name: "prod-us",
    version: "v1.33.5",
    nodes: &["pool-general-q4m8d", "pool-general-q4m8f"],
    apps: &[
        (&CHECKOUT, 3, 2 * DAY + 3 * HOUR),
        (&CART, 2, 6 * DAY),
        (&REDIS, 1, 29 * DAY),
        (&PAYMENTS, 2, 2 * DAY + 3 * HOUR),
        (&POSTGRES, 2, 58 * DAY),
        (&INDEXER, 1, 9 * HOUR),
        (&PROMETHEUS, 1, 15 * DAY),
        (&NODE_EXPORTER, 2, 15 * DAY),
        (&COREDNS, 2, 58 * DAY),
        (&METRICS_SERVER, 1, 58 * DAY),
    ],
    live: false,
};

pub(super) const STAGING: Site = Site {
    name: "staging",
    version: "v1.34.1",
    nodes: &["pool-small-h9t2c"],
    apps: &[
        (&CHECKOUT_NEXT, 1, 52 * MINUTE),
        (&CART, 1, 3 * DAY),
        (&REDIS, 1, 12 * DAY),
        (&PAYMENTS, 1, DAY + 2 * HOUR),
        (&POSTGRES, 1, 12 * DAY),
        (&INDEXER, 1, 3 * HOUR),
        (&COREDNS, 1, 40 * DAY),
        (&METRICS_SERVER, 1, 40 * DAY),
    ],
    live: true,
};

/// The node of prod-eu that the drain empties.
pub(super) const DRAINED: &str = "pool-general-7x2kw";
/// The Job of the CronJob of today, at 02:00: its name has the minute of its start.
pub(super) const REPORT_JOB: &str = "nightly-report-29999640";
/// Why the scheduler finds no node for the new pod of cart.
pub(super) const PENDING: &str = "0/3 nodes are available: 1 node(s) were unschedulable, 2 node(s) didn't match pod \
                                  anti-affinity rules. preemption: 0/3 nodes are available: 3 Preemption is not \
                                  helpful for scheduling.";

/// A pod that differs from the rule of its app: its state, its age and its node.
struct Odd {
    state: State,
    age: Option<u64>,
    node: Option<&'static str>,
}

/// The pod `index` of an app in a cluster, when it differs from the rule of its app.
fn odd(site: &str, app: &str, index: u32) -> Option<Odd> {
    let odd = |state, age, node| Some(Odd { state, age, node });
    match (site, app, index) {
        // The new pods of the drain: one waits for a node.
        ("prod-eu", "cart", 2) => odd(State::Pending(PENDING), Some(70), None),
        ("prod-eu", "grafana", 0) => odd(State::Running(0, 0), Some(75), None),
        ("prod-eu", "coredns", 1) => odd(State::Running(0, 0), Some(80), None),
        ("prod-eu", "postgres", 1) => odd(State::Running(1, 3 * DAY + 2 * HOUR), None, Some(DRAINED)),
        ("prod-eu", "redis", 0) => odd(State::Running(2, 9 * DAY), None, None),
        ("prod-eu", "search-indexer", 0) => odd(State::Running(1, 2 * HOUR + 14 * MINUTE), None, None),
        ("prod-us", "payments-worker", 1) => odd(State::Running(3, 5 * HOUR), None, None),
        ("staging", "checkout-api", 0) => odd(State::CrashLoopBackOff(14, 95), None, None),
        _ => None,
    }
}

/// The name of pod `index` of an app and the kind and name of its controller.
fn names(site: &str, app: &App, index: u32) -> (String, &'static str, String) {
    let end = suffix(&[site, app.name, &index.to_string()], 5);
    match app.owner {
        Owner::Deployment(hash) => {
            let set = format!("{}-{hash}", app.name);
            (format!("{set}-{end}"), "ReplicaSet", set)
        }
        Owner::StatefulSet => (format!("{}-{index}", app.name), "StatefulSet", app.name.to_owned()),
        Owner::DaemonSet => (format!("{}-{end}", app.name), "DaemonSet", app.name.to_owned()),
        Owner::CronJob => (format!("{REPORT_JOB}-{end}"), "Job", REPORT_JOB.to_owned()),
    }
}

/// The pods of an app in a cluster, with their app.
pub(super) fn pods_of(site: &Site, app: &'static App, count: u32, age: u64) -> Vec<(Arc<Pod>, &'static App)> {
    // The scheduler puts no new pod on the node of the drain. The pods of each app start on
    // an other node, so the nodes have mixed pods.
    let open: Vec<&str> = site.nodes.iter().copied().filter(|node| *node != DRAINED).collect();
    let start = app.name.len() + app.namespace.len();
    (0..count)
        .map(|index| {
            let odd = odd(site.name, app.name, index);
            let (name, kind, owner) = names(site.name, app, index);
            let node = match (&odd, app.owner) {
                (Some(Odd { state: State::Pending(_), .. }), _) => None,
                (Some(Odd { node: Some(node), .. }), _) => Some(*node),
                (_, Owner::DaemonSet) => site.nodes.get(index as usize).copied(),
                _ => open.get((start + index as usize) % open.len()).copied(),
            };
            let state = match (&odd, app.owner) {
                (Some(odd), _) => odd.state,
                (None, Owner::CronJob) => State::Completed,
                (None, _) => State::Running(0, 0),
            };
            let age = odd.and_then(|odd| odd.age).unwrap_or(age - u64::from(index) * 11);
            let namespace = app.namespace;
            let owner = (kind, owner.clone(), uid(&[site.name, kind, namespace, &owner]));
            let plan = PodPlan { cluster: site.name, app, name, node, age, state, owner };
            (object::<Pod>(pod(&plan)), app)
        })
        .collect()
}
