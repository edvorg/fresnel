//! The objects of each kind of one cluster of the showcase: the apps with their pods and
//! controllers, the nodes, the Services, the ConfigMaps, the Secrets and the warnings.

use std::sync::Arc;

use k8s_openapi::api::apps::v1::{DaemonSet, Deployment, ReplicaSet, StatefulSet};
use k8s_openapi::api::batch::v1::{CronJob, Job};
use k8s_openapi::api::core::v1::{ConfigMap, ContainerStatus, Event, Namespace, Node, Pod, Secret, Service};
use serde_json::{Value, json};

use super::objects::{
    App, DAY, Owner, Warning, conditions, cron_job, event, meta, node, object, replica_set, service, workload,
};
use super::world::{DRAINED, NAMESPACES, PENDING, REPORT_JOB, Site, pods_of};

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

/// The objects of each kind of one cluster.
#[derive(Default)]
pub(super) struct Stores {
    pub(super) pods: Vec<(Arc<Pod>, &'static App)>,
    pub(super) deployments: Vec<Arc<Deployment>>,
    pub(super) replica_sets: Vec<Arc<ReplicaSet>>,
    pub(super) stateful_sets: Vec<Arc<StatefulSet>>,
    pub(super) daemon_sets: Vec<Arc<DaemonSet>>,
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
        for (app, count, age) in site.apps {
            let pods = pods_of(site, app, *count, *age);
            stores.controllers(site, app, (*count, ready(&pods)), *age);
            stores.pods.extend(pods);
        }
        let cluster = site.name;
        for name in site.nodes {
            stores.nodes.push(object(node(cluster, name, site.version, 61 * DAY, *name == DRAINED)));
        }
        for namespace in NAMESPACES {
            stores.namespaces.push(object(json!({"metadata": meta(cluster, "Namespace", "", namespace, 90 * DAY),
                "status": {"phase": "Active"}})));
        }
        let apps = site.apps.iter().map(|(app, _, _)| app.name).collect::<Vec<_>>();
        for (namespace, name, app, port) in SERVICES.into_iter().filter(|service| apps.contains(&service.2)) {
            stores.services.push(object(service(cluster, (namespace, name, app), port)));
        }
        for (namespace, name) in CONFIG_MAPS {
            let config_map = json!({"metadata": meta(cluster, "ConfigMap", namespace, name, 40 * DAY), "data": {}});
            stores.config_maps.push(object(config_map));
        }
        for (namespace, name, kind, keys) in SECRETS {
            let data: serde_json::Map<String, Value> = keys.iter().map(|key| ((*key).to_owned(), "".into())).collect();
            let secret = json!({"metadata": meta(cluster, "Secret", namespace, name, 40 * DAY), "type": kind,
                "data": data});
            stores.secrets.push(object(secret));
        }
        stores.warnings(cluster);
        stores
    }

    /// The controllers of the pods of an app: their workload and its ReplicaSet or Job.
    fn controllers(&mut self, site: &Site, app: &App, pods: (u32, u32), age: u64) {
        let cluster = site.name;
        match app.owner {
            Owner::Deployment(hash) => {
                let mut deployment = workload(cluster, app, "Deployment", pods, 41 * DAY);
                deployment["status"]["conditions"] = conditions(pods.0 == pods.1, age);
                self.replica_sets.push(object(replica_set(cluster, app, &deployment, hash, pods)));
                self.deployments.push(object(deployment));
            }
            Owner::StatefulSet => {
                self.stateful_sets.push(object(workload(cluster, app, "StatefulSet", pods, 61 * DAY)))
            }
            Owner::DaemonSet => self.daemon_sets.push(object(workload(cluster, app, "DaemonSet", pods, 61 * DAY))),
            Owner::CronJob => {
                let (cron, job) = cron_job(cluster, app, REPORT_JOB, age);
                self.cron_jobs.push(object(cron));
                self.jobs.push(object(job));
            }
        }
    }

    /// The warnings of the pods that are not healthy: a pod with no node and a pod whose
    /// container crashes.
    fn warnings(&mut self, cluster: &str) {
        for (pod, app) in &self.pods {
            let status = pod.status.as_ref();
            let phase = status.and_then(|status| status.phase.as_deref());
            let crashes = status.and_then(|status| status.container_statuses.as_ref()).is_some_and(|statuses| {
                let waits = |status: &ContainerStatus| {
                    let waiting = status.state.as_ref().and_then(|state| state.waiting.as_ref());
                    waiting.and_then(|waiting| waiting.reason.clone())
                };
                statuses.iter().filter_map(waits).any(|reason| reason == "CrashLoopBackOff")
            });
            let meta = meta_of(pod);
            if phase == Some("Pending") {
                let warning = Warning { reason: "FailedScheduling", message: PENDING, count: 4, last: 12 };
                self.events.push(object(event(cluster, &meta, &warning)));
            } else if crashes {
                let (name, uid) = (pod.metadata.name.as_deref(), pod.metadata.uid.as_deref());
                let back_off = format!(
                    "Back-off restarting failed container {} in pod {}_{}({})",
                    app.name,
                    name.unwrap_or_default(),
                    app.namespace,
                    uid.unwrap_or_default()
                );
                let address = status.and_then(|status| status.pod_ip.as_deref()).unwrap_or_default();
                let probe = format!(
                    "Readiness probe failed: Get \"http://{address}:8080/ready\": dial tcp {address}:8080: connect: \
                     connection refused"
                );
                let crash = Warning { reason: "BackOff", message: &back_off, count: 212, last: 31 };
                let probe = Warning { reason: "Unhealthy", message: &probe, count: 41, last: 96 };
                self.events.push(object(event(cluster, &meta, &crash)));
                self.events.push(object(event(cluster, &meta, &probe)));
            }
        }
    }
}

/// The count of the pods whose containers are each ready.
fn ready(pods: &[(Arc<Pod>, &App)]) -> u32 {
    let is_ready = |pod: &Arc<Pod>| {
        let statuses = pod.status.as_ref().and_then(|status| status.container_statuses.as_ref());
        statuses.is_some_and(|statuses| statuses.iter().all(|status| status.ready))
    };
    pods.iter().filter(|(pod, _)| is_ready(pod)).count() as u32
}

/// The metadata of a pod as JSON, for an Event about it.
fn meta_of(pod: &Pod) -> Value {
    serde_json::to_value(&pod.metadata).expect("JSON")
}
