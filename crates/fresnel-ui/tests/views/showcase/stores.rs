//! The objects of each kind of one cluster of the showcase: the apps with their pods,
//! controllers and disruption budgets, the nodes, the Services, the ConfigMaps, the Secrets
//! and the Events of the last hour.

use std::sync::Arc;

use k8s_openapi::api::apps::v1::{DaemonSet, Deployment, ReplicaSet, StatefulSet};
use k8s_openapi::api::batch::v1::{CronJob, Job};
use k8s_openapi::api::core::v1::{ConfigMap, Event, Namespace, Node, Pod, Secret, Service};
use k8s_openapi::api::policy::v1::PodDisruptionBudget;
use serde_json::{Value, json};

use super::objects::{
    App, DAY, HOUR, Note, Owner, State, conditions, cron_job, event, meta, node, object, replica_set, service, workload,
};
use super::world::{CORDONED, DRAINED, NAMESPACES, Planned, REPORT_JOB, Site, aged, earlier, pods_of, rollout};

/// The Services of the apps: namespace, name, app and port.
const SERVICES: [(&str, &str, &str, u16); 7] = [
    ("shop", "checkout-api", "checkout-api", 8080),
    ("shop", "cart", "cart", 8080),
    ("shop", "redis", "redis", 6379),
    ("payments", "postgres", "postgres", 5432),
    ("monitoring", "prometheus", "prometheus", 9090),
    ("monitoring", "grafana", "grafana", 3000),
    ("kube-system", "kube-dns", "coredns", 53),
];
const CONFIG_MAPS: [(&str, &str); 5] = [
    ("shop", "checkout-api-config"),
    ("shop", "cart-config"),
    ("payments", "report-settings"),
    ("monitoring", "prometheus-config"),
    ("kube-system", "coredns"),
];
/// The Secrets with their type and keys. A store has no value of a Secret.
const SECRETS: [(&str, &str, &str, &[&str]); 4] = [
    ("payments", "postgres-credentials", "Opaque", &["password", "url", "username"]),
    ("shop", "checkout-api-session", "Opaque", &["signing-key"]),
    ("shop", "redis-auth", "Opaque", &["password"]),
    ("monitoring", "grafana-admin", "Opaque", &["admin-password", "admin-user"]),
];
/// The apps with a PodDisruptionBudget that keeps each of their pods: a drain waits for it.
const BUDGETS: [&str; 1] = ["postgres"];

/// The objects of each kind of one cluster.
#[derive(Default)]
pub(super) struct Stores {
    pub(super) pods: Vec<Planned>,
    pub(super) deployments: Vec<Arc<Deployment>>,
    pub(super) replica_sets: Vec<Arc<ReplicaSet>>,
    pub(super) stateful_sets: Vec<Arc<StatefulSet>>,
    pub(super) daemon_sets: Vec<Arc<DaemonSet>>,
    pub(super) budgets: Vec<Arc<PodDisruptionBudget>>,
    pub(super) jobs: Vec<Arc<Job>>,
    pub(super) cron_jobs: Vec<Arc<CronJob>>,
    pub(super) services: Vec<Arc<Service>>,
    pub(super) nodes: Vec<Arc<Node>>,
    pub(super) namespaces: Vec<Arc<Namespace>>,
    pub(super) events: Vec<Arc<Event>>,
    pub(super) config_maps: Vec<Arc<ConfigMap>>,
    pub(super) secrets: Vec<Arc<Secret>>,
}

impl Stores {
    /// The objects of the cluster of a site.
    pub(super) fn of(site: &Site) -> Self {
        let mut stores = Self::default();
        let cluster = site.name;
        // The workloads and their Services, ConfigMaps and Secrets came two days after the
        // namespaces.
        let created =
            |kind: &str, namespace: &str, name: &str| aged(site.age - 2 * DAY, &[cluster, kind, namespace, name]);
        for (app, count, age) in site.apps {
            let rollout = rollout(site, app, *age);
            let pods = pods_of(site, app, *count, rollout);
            let ready = pods.iter().filter(|pod| matches!(pod.state, State::Running(..))).count() as u32;
            let workload = created("workload", app.namespace, app.name);
            stores.controllers(site, app, (*count, ready), (rollout, workload));
            if BUDGETS.contains(&app.name) {
                stores.budgets.push(object(budget(cluster, app, (*count, ready), workload)));
            }
            stores.pods.extend(pods);
        }
        stores.nodes(site);
        for namespace in NAMESPACES {
            let age = aged(site.age, &[cluster, "Namespace", namespace]);
            stores.namespaces.push(object(json!({"metadata": meta(cluster, "Namespace", "", namespace, age),
                "status": {"phase": "Active"}})));
        }
        let apps = site.apps.iter().map(|(app, _, _)| app.name).collect::<Vec<_>>();
        for (namespace, name, app, port) in SERVICES.into_iter().filter(|service| apps.contains(&service.2)) {
            let age = created("Service", namespace, name);
            stores.services.push(object(service(cluster, (namespace, name, app), port, age)));
        }
        for (namespace, name) in CONFIG_MAPS {
            let age = created("ConfigMap", namespace, name);
            let config_map = json!({"metadata": meta(cluster, "ConfigMap", namespace, name, age), "data": {}});
            stores.config_maps.push(object(config_map));
        }
        for (namespace, name, kind, keys) in SECRETS {
            let data: serde_json::Map<String, Value> = keys.iter().map(|key| ((*key).to_owned(), "".into())).collect();
            let age = created("Secret", namespace, name);
            let secret = json!({"metadata": meta(cluster, "Secret", namespace, name, age), "type": kind,
                "data": data});
            stores.secrets.push(object(secret));
        }
        stores.warnings(cluster);
        stores
    }

    /// The nodes of a cluster, and the Event of the cordon of the node of the drain.
    fn nodes(&mut self, site: &Site) {
        for name in site.nodes {
            let age = aged(site.node_age, &[site.name, "Node", name]);
            let node = node(site.name, (name, site.network), site.version, age, *name == DRAINED);
            if *name == DRAINED {
                let message = format!("Node {name} status is now: NodeNotSchedulable");
                let note =
                    Note { kind: "Normal", reason: "NodeNotSchedulable", message: &message, count: 1, last: CORDONED };
                self.events.push(object(event(site.name, ("Node", &node["metadata"]), &note)));
            }
            self.nodes.push(object(node));
        }
    }

    /// The controllers of the pods of an app: their workload and its ReplicaSets or Job.
    /// `ages` are the ages of the last rollout and of the workload.
    fn controllers(&mut self, site: &Site, app: &App, pods: (u32, u32), (rollout, created): (u64, u64)) {
        let cluster = site.name;
        match app.owner {
            Owner::Deployment(hash, revision) => {
                let mut deployment = workload(cluster, app, "Deployment", pods, created);
                deployment["status"]["conditions"] = conditions(pods.0 == pods.1, rollout);
                let earlier = earlier(app, rollout, created);
                for set in &earlier {
                    let mut old = replica_set(cluster, app, &deployment, (&set.hash, set.revision, set.age), (0, 0));
                    old["spec"]["template"]["spec"]["containers"][0]["image"] = set.image.into();
                    self.replica_sets.push(object(old));
                }
                self.replica_sets.push(object(replica_set(cluster, app, &deployment, (hash, revision, rollout), pods)));
                // An Event lives one hour: only a rollout of the last hour still has its own.
                if let Some(old) = earlier.last().filter(|_| rollout < HOUR) {
                    self.scaled(
                        cluster,
                        &deployment,
                        [(&old.hash, pods.0, 0, rollout), (hash, 0, pods.0, rollout.saturating_sub(1))],
                    );
                }
                self.deployments.push(object(deployment));
            }
            Owner::StatefulSet => self.stateful_sets.push(object(workload(cluster, app, "StatefulSet", pods, created))),
            Owner::DaemonSet => self.daemon_sets.push(object(workload(cluster, app, "DaemonSet", pods, created))),
            Owner::CronJob => {
                let (cron, job) = cron_job(cluster, app, (REPORT_JOB, rollout), created);
                self.cron_jobs.push(object(cron));
                self.jobs.push(object(job));
            }
        }
    }

    /// The Events of a rollout that stops the pods of the old ReplicaSet first: each change of
    /// the replicas of a ReplicaSet, from and to, with its age.
    fn scaled(&mut self, cluster: &str, deployment: &Value, changes: [(&str, u32, u32, u64); 2]) {
        let name = deployment["metadata"]["name"].as_str().unwrap_or_default();
        for (hash, from, to, last) in changes {
            let way = if to < from { "down" } else { "up" };
            let message = format!("Scaled {way} replica set {name}-{hash} from {from} to {to}");
            let note = Note { kind: "Normal", reason: "ScalingReplicaSet", message: &message, count: 1, last };
            self.events.push(object(event(cluster, ("Deployment", &deployment["metadata"]), &note)));
        }
    }

    /// The warnings of the pods that are not healthy: a pod with no node and a pod whose
    /// container crashes.
    fn warnings(&mut self, cluster: &str) {
        for Planned { pod, app, state } in &self.pods {
            let meta = meta_of(pod);
            let warning = |reason, message, count, last| Note { kind: "Warning", reason, message, count, last };
            match state {
                State::Pending(reason) => {
                    let warning = warning("FailedScheduling", *reason, 4, 12);
                    self.events.push(object(event(cluster, ("Pod", &meta), &warning)));
                }
                State::CrashLoopBackOff(..) => {
                    let (name, uid) = (pod.metadata.name.as_deref(), pod.metadata.uid.as_deref());
                    let back_off = format!(
                        "Back-off restarting failed container {} in pod {}_{}({})",
                        app.name,
                        name.unwrap_or_default(),
                        app.namespace,
                        uid.unwrap_or_default()
                    );
                    let address = pod.status.as_ref().and_then(|status| status.pod_ip.as_deref()).unwrap_or_default();
                    let probe = format!(
                        "Readiness probe failed: Get \"http://{address}:8080/ready\": dial tcp {address}:8080: \
                         connect: connection refused"
                    );
                    let crash = warning("BackOff", &back_off, 212, 31);
                    let probe = warning("Unhealthy", &probe, 41, 96);
                    self.events.push(object(event(cluster, ("Pod", &meta), &crash)));
                    self.events.push(object(event(cluster, ("Pod", &meta), &probe)));
                }
                State::Running(..) | State::Completed => {}
            }
        }
    }
}

/// A PodDisruptionBudget of an app that keeps each of its pods, with the count of its pods
/// and of the ready ones: it allows no eviction.
fn budget(cluster: &str, app: &App, (pods, ready): (u32, u32), age: u64) -> Value {
    json!({
        "metadata": meta(cluster, "PodDisruptionBudget", app.namespace, app.name, age),
        "spec": {"minAvailable": pods, "selector": {"matchLabels": {"app": app.name}}},
        "status": {"currentHealthy": ready, "desiredHealthy": pods, "expectedPods": pods,
            "disruptionsAllowed": ready.saturating_sub(pods)},
    })
}

/// The metadata of a pod as JSON, for an Event about it.
fn meta_of(pod: &Pod) -> Value {
    serde_json::to_value(&pod.metadata).expect("JSON")
}
