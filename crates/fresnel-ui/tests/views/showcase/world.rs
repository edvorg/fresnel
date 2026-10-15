//! The world of the showcase: one shop with three clusters, each name made up.
//!
//! prod-eu and staging are live. prod-us lost its connection 48 s ago, so each of its stores
//! keeps its objects as old ones. A drain of the node general-bw9fd of prod-eu runs: it
//! evicted the pods of cart, grafana and coredns there, whose new pods run on the other
//! nodes or wait for one, and a disruption budget holds postgres-1. On staging, a release
//! candidate of checkout-api crashes.

use std::sync::Arc;

use k8s_openapi::api::core::v1::Pod;

use super::objects::{App, DAY, HOUR, MINUTE, Owner, PodPlan, State, hash, object, pod, suffix, uid};

/// The hash of the template of the release of checkout-api that prod runs.
const RELEASE: &str = "7d9c5b8f6";
pub(super) const CHECKOUT: App = App {
    namespace: "shop",
    name: "checkout-api",
    owner: Owner::Deployment(RELEASE, 7),
    image: "shop/checkout-api:1.42.3",
    requests: ["200m", "256Mi"],
    limits: [Some("500m"), Some("512Mi")],
    usage: [190.0, 300.0],
};
/// The revision of the release candidate of checkout-api on staging, which crashes. Its
/// revisions 6 and 7 are the release of prod and the candidate before.
pub(super) const RC_REVISION: u32 = 8;
/// The age of the rollout of that release candidate.
pub(super) const RC_ROLLOUT: u64 = 52 * MINUTE;
/// The ages of the revisions 6 and 7 on staging, and the hash and the image of revision 7.
pub(super) const RELEASE_AGE: u64 = 3 * DAY + 2 * HOUR + 41 * MINUTE;
pub(super) const CANDIDATE_AGE: u64 = DAY + 5 * HOUR + 9 * MINUTE;
pub(super) const CANDIDATE: (&str, &str) = ("8b5d7c9f4", "shop/checkout-api:1.43.0-rc.0");
/// That release candidate: a new image with the resources of the release.
pub(super) const CHECKOUT_NEXT: App =
    App { owner: Owner::Deployment("59f7c4d8b", RC_REVISION), image: "shop/checkout-api:1.43.0-rc.1", ..CHECKOUT };
const CART: App = App {
    namespace: "shop",
    name: "cart",
    owner: Owner::Deployment("5c8f7b9d6", 12),
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
    owner: Owner::Deployment("6b4d8c7f9", 23),
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
    owner: Owner::Deployment("84f6d9b5c", 4),
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
    owner: Owner::Deployment("7f5b6c8d4", 3),
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
    owner: Owner::Deployment("5d78c9869d", 1),
    image: "coredns/coredns:1.11.4",
    requests: ["100m", "70Mi"],
    limits: [None, Some("170Mi")],
    usage: [6.0, 28.0],
};
const METRICS_SERVER: App = App {
    namespace: "kube-system",
    name: "metrics-server",
    owner: Owner::Deployment("6f8c7b9d5", 2),
    image: "registry.k8s.io/metrics-server/metrics-server:v0.7.2",
    requests: ["100m", "200Mi"],
    limits: [None, None],
    usage: [9.0, 34.0],
};

/// The namespaces of each cluster.
pub(super) const NAMESPACES: [&str; 6] = ["default", "kube-system", "monitoring", "payments", "search", "shop"];

/// One cluster: its name, the version of its server, its ages, its nodes, and its apps with
/// the count of their pods and the age of their last rollout.
pub(super) struct Site {
    pub(super) name: &'static str,
    pub(super) version: &'static str,
    /// The age of the cluster: its namespaces are that old, its workloads two days less.
    pub(super) age: u64,
    /// The age of its nodes: more than the age of each pod on them.
    pub(super) node_age: u64,
    /// The network of the nodes: 10.<network>.0.0/16, one of each cluster.
    pub(super) network: u8,
    pub(super) nodes: &'static [&'static str],
    pub(super) apps: &'static [(&'static App, u32, u64)],
    /// False for a cluster that lost its connection: its stores keep old objects.
    pub(super) live: bool,
}

pub(super) const PROD_EU: Site = Site {
    name: "prod-eu",
    version: "v1.33.5",
    age: 92 * DAY,
    node_age: 64 * DAY,
    network: 0,
    nodes: &["general-7x2kp", "general-m4tqz", DRAINED],
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
    age: 80 * DAY,
    node_age: 59 * DAY,
    network: 1,
    nodes: &["general-q4m8d", "general-hz62c"],
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
    age: 47 * DAY,
    node_age: 41 * DAY,
    network: 2,
    nodes: &["small-h9t2c"],
    apps: &[
        (&CHECKOUT_NEXT, 1, RC_ROLLOUT),
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

/// The node of prod-eu that the drain empties, and the seconds since the drain cordoned it:
/// the new pods of its evicted ones are younger.
pub(super) const DRAINED: &str = "general-bw9fd";
pub(super) const CORDONED: u64 = 84;
/// The Job of the CronJob of today, at 02:00: its name has the minute of its start.
pub(super) const REPORT_JOB: &str = "nightly-report-29999640";
/// Why the scheduler finds no node for the new pod of cart.
const PENDING: &str = "0/3 nodes are available: 1 node(s) were unschedulable, 2 node(s) didn't match pod \
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
        ("prod-eu", "postgres", 1) => odd(State::Running(1, 3 * DAY + 2 * HOUR + 17 * MINUTE), None, Some(DRAINED)),
        ("prod-eu", "redis", 0) => odd(State::Running(2, 9 * DAY + 6 * HOUR + 41 * MINUTE), None, None),
        ("prod-eu", "search-indexer", 0) => odd(State::Running(1, 2 * HOUR + 14 * MINUTE + 33), None, None),
        ("prod-us", "payments-worker", 1) => odd(State::Running(3, 5 * HOUR + 23 * MINUTE + 8), None, None),
        ("staging", "checkout-api", 0) => odd(State::CrashLoopBackOff(14, 95), None, None),
        _ => None,
    }
}

/// The name of pod `index` of an app and the kind and name of its controller.
fn names(site: &str, app: &App, index: u32) -> (String, &'static str, String) {
    let end = suffix(&[site, app.name, &index.to_string()], 5);
    match app.owner {
        Owner::Deployment(hash, _) => {
            let set = format!("{}-{hash}", app.name);
            (format!("{set}-{end}"), "ReplicaSet", set)
        }
        Owner::StatefulSet => (format!("{}-{index}", app.name), "StatefulSet", app.name.to_owned()),
        Owner::DaemonSet => (format!("{}-{end}", app.name), "DaemonSet", app.name.to_owned()),
        Owner::CronJob => (format!("{REPORT_JOB}-{end}"), "Job", REPORT_JOB.to_owned()),
    }
}

/// The age of an object with minutes and seconds of its own, as its place gives them: an
/// age of whole hours or days looks made up. The part is less than a twentieth of the age
/// and less than a day, so a pod stays younger than its node and its workload.
pub(super) fn aged(age: u64, place: &[&str]) -> u64 {
    age + hash(place) % (age / 20).clamp(1, DAY)
}

/// The age of the last rollout of an app in a cluster: `age` with the minutes and seconds of
/// its place. A CronJob starts its Job on the minute of its schedule.
pub(super) fn rollout(site: &Site, app: &App, age: u64) -> u64 {
    match app.owner {
        Owner::CronJob => age,
        _ => aged(age, &[site.name, app.name, "rollout"]),
    }
}

/// An earlier ReplicaSet of a Deployment: the hash of its template, its image, its revision
/// and its age. A Deployment keeps it with no pod, for a rollback.
pub(super) struct Earlier {
    pub(super) hash: String,
    pub(super) image: &'static str,
    pub(super) revision: u32,
    pub(super) age: u64,
}

/// The earlier ReplicaSets of an app, the oldest first: the release and the first candidate
/// before the release candidate of staging, and the revision before for each other
/// Deployment. `created` and `rollout` are the ages of the workload and its last rollout.
pub(super) fn earlier(app: &App, rollout: u64, created: u64) -> Vec<Earlier> {
    let Owner::Deployment(_, revision) = app.owner else {
        return Vec::new();
    };
    let earlier = |hash: &str, image, revision, age| Earlier { hash: hash.to_owned(), image, revision, age };
    if app.image == CHECKOUT_NEXT.image {
        let (hash, image) = CANDIDATE;
        return vec![
            earlier(RELEASE, CHECKOUT.image, RC_REVISION - 2, RELEASE_AGE),
            earlier(hash, image, RC_REVISION - 1, CANDIDATE_AGE),
        ];
    }
    let hash = suffix(&[app.name, "earlier"], 9);
    (revision > 1).then(|| earlier(&hash, app.image, revision - 1, (created + rollout) / 2)).into_iter().collect()
}

/// The pods of a rollout start this many seconds apart, the first one when it starts.
const STARTS_APART: u64 = 11;

/// A pod of the world with its app and its state.
pub(super) struct Planned {
    pub(super) pod: Arc<Pod>,
    pub(super) app: &'static App,
    pub(super) state: State,
}

/// The pods of an app in a cluster whose last rollout was `rollout` seconds ago.
pub(super) fn pods_of(site: &Site, app: &'static App, count: u32, rollout: u64) -> Vec<Planned> {
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
            let age = odd.and_then(|odd| odd.age).unwrap_or(rollout.saturating_sub(u64::from(index) * STARTS_APART));
            let namespace = app.namespace;
            let owner = (kind, owner.clone(), uid(&[site.name, kind, namespace, &owner]));
            let plan = PodPlan { cluster: site.name, app, name, node, age, state, owner };
            Planned { pod: object::<Pod>(pod(&plan)), app, state }
        })
        .collect()
}
