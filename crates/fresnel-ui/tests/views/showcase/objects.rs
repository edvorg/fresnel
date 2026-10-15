//! The objects of the showcase in the JSON of the API server, with the fields that the views
//! read. Each name, UID and IP address comes from a hash of its place, so two runs give the
//! same objects; each time is a span before the time of the view tests.

use std::sync::Arc;

use jiff::{SignedDuration, Timestamp};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::fake::now;

pub(super) const MINUTE: u64 = 60;
pub(super) const HOUR: u64 = 3_600;
pub(super) const DAY: u64 = 86_400;

/// The controller of the pods of an app. A Deployment names the hash of its pod template.
#[derive(Clone, Copy)]
pub(super) enum Owner {
    Deployment(&'static str),
    StatefulSet,
    DaemonSet,
    CronJob,
}

/// One app: a workload with the resources and the usage of each of its pods.
pub(super) struct App {
    pub(super) namespace: &'static str,
    pub(super) name: &'static str,
    pub(super) owner: Owner,
    pub(super) image: &'static str,
    /// CPU and memory, as a manifest writes them.
    pub(super) requests: [&'static str; 2],
    pub(super) limits: [Option<&'static str>; 2],
    /// The usage of one pod at the last poll: millicores and MiB.
    pub(super) usage: [f64; 2],
}

/// The state of one pod, as `kubectl get pods` prints it.
#[derive(Clone, Copy)]
pub(super) enum State {
    /// With the count of restarts and the age of the last one.
    Running(u32, u64),
    CrashLoopBackOff(u32, u64),
    /// No node fits: the reason of the scheduler.
    Pending(&'static str),
    Completed,
}

/// One pod of an app: its cluster, name, node, age in seconds and state.
pub(super) struct PodPlan<'a> {
    pub(super) cluster: &'a str,
    pub(super) app: &'a App,
    pub(super) name: String,
    pub(super) node: Option<&'a str>,
    pub(super) age: u64,
    pub(super) state: State,
    /// The kind, the name and the UID of the controller of the pod.
    pub(super) owner: (&'a str, String, String),
}

/// A hash of the place of an object: FNV-1a, which has no seed of the process.
pub(super) fn hash(parts: &[&str]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in parts.iter().flat_map(|part| part.bytes().chain([0])) {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// The UID of an object with this place.
pub(super) fn uid(parts: &[&str]) -> String {
    let (high, low) = (hash(parts), hash(&[parts, &["low"]].concat()));
    let text = format!("{high:016x}{low:016x}");
    let part = |range: std::ops::Range<usize>| text.get(range).unwrap_or_default().to_owned();
    format!("{}-{}-4{}-8{}-{}", part(0..8), part(8..12), part(13..16), part(17..20), part(20..32))
}

/// The random end of a name, from the letters that Kubernetes uses for one.
pub(super) fn suffix(parts: &[&str], length: usize) -> String {
    const LETTERS: &[u8] = b"bcdfghjklmnpqrstvwxz2456789";
    let mut seed = hash(parts);
    (0..length)
        .map(|_| {
            let letter = LETTERS[(seed % LETTERS.len() as u64) as usize];
            seed /= LETTERS.len() as u64;
            char::from(letter)
        })
        .collect()
}

/// The time `seconds` before the time of the view tests, as the API writes it.
pub(super) fn ago(seconds: u64) -> String {
    let now = Timestamp::try_from(now()).expect("a time");
    (now - SignedDuration::from_secs(seconds as i64)).to_string()
}

/// The object of a kind with this JSON.
pub(super) fn object<K: DeserializeOwned>(value: Value) -> Arc<K> {
    Arc::new(serde_json::from_value(value).expect("an object of the showcase"))
}

/// The metadata of an object of a cluster, with its UID.
pub(super) fn meta(cluster: &str, kind: &str, namespace: &str, name: &str, age: u64) -> Value {
    let uid = uid(&[cluster, kind, namespace, name]);
    let mut meta = json!({"name": name, "uid": uid, "creationTimestamp": ago(age), "resourceVersion": "48211"});
    if !namespace.is_empty() {
        meta["namespace"] = namespace.into();
    }
    meta
}

/// The reference of an object to its controller.
fn owner_reference(kind: &str, name: &str, uid: &str) -> Value {
    let api = match kind {
        "Job" | "CronJob" => "batch/v1",
        _ => "apps/v1",
    };
    json!([{"apiVersion": api, "kind": kind, "name": name, "uid": uid, "controller": true}])
}

/// The template of the pods of an app, with its resources.
fn template(app: &App) -> Value {
    let [cpu, memory] = app.requests;
    let mut limits = serde_json::Map::new();
    for (resource, limit) in ["cpu", "memory"].into_iter().zip(app.limits) {
        if let Some(limit) = limit {
            limits.insert(resource.to_owned(), limit.into());
        }
    }
    let resources = json!({"requests": {"cpu": cpu, "memory": memory}, "limits": limits});
    json!({
        "metadata": {"labels": {"app": app.name}},
        "spec": {"containers": [{"name": app.name, "image": app.image, "resources": resources}]},
    })
}

/// A pod as the API server gives it in each state of the showcase.
pub(super) fn pod(plan: &PodPlan<'_>) -> Value {
    let PodPlan { cluster, app, name, node, age, state, owner } = plan;
    let mut pod = template(app);
    pod["metadata"] = meta(cluster, "Pod", app.namespace, name, *age);
    pod["metadata"]["labels"] = json!({"app": app.name});
    pod["metadata"]["ownerReferences"] = owner_reference(owner.0, &owner.1, &owner.2);
    let ip = hash(&[cluster, name]);
    let started = ago(age.saturating_sub(4));
    let container = |ready: bool, restarts: u32, state: Value, last: Value| {
        json!([{"name": app.name, "image": app.image, "imageID": "", "ready": ready, "started": ready,
            "restartCount": restarts, "state": state, "lastState": last}])
    };
    let ended = |seconds: u64, code: i32, reason: &str| {
        let run = json!({"exitCode": code, "reason": reason, "finishedAt": ago(seconds)});
        json!({ "terminated": run })
    };
    let condition = |ready: bool| json!([{"type": "Ready", "status": if ready { "True" } else { "False" }}]);
    let status = match *state {
        State::Running(restarts, last) => {
            let last = if restarts == 0 { json!({}) } else { ended(last, 137, "OOMKilled") };
            let running = json!({"running": {"startedAt": started}});
            json!({"phase": "Running", "conditions": condition(true),
                "containerStatuses": container(true, restarts, running, last)})
        }
        State::CrashLoopBackOff(restarts, last) => {
            let waiting = json!({"waiting": {"reason": "CrashLoopBackOff"}});
            json!({"phase": "Running", "conditions": condition(false),
                "containerStatuses": container(false, restarts, waiting, ended(last, 1, "Error"))})
        }
        State::Pending(reason) => json!({"phase": "Pending", "conditions": [
            {"type": "PodScheduled", "status": "False", "reason": "Unschedulable", "message": reason}]}),
        State::Completed => json!({"phase": "Succeeded", "conditions": condition(false),
            "containerStatuses": container(false, 0, ended(age - 95, 0, "Completed"), json!({}))}),
    };
    pod["status"] = status;
    if let Some(node) = node {
        pod["spec"]["nodeName"] = (*node).into();
        pod["status"]["podIP"] = format!("10.244.{}.{}", ip % 8, 2 + ip % 250).into();
        pod["status"]["startTime"] = started.into();
    }
    pod
}

/// A workload of an app with the count of its pods that are ready.
pub(super) fn workload(cluster: &str, app: &App, kind: &str, (replicas, ready): (u32, u32), age: u64) -> Value {
    let mut workload = json!({
        "metadata": meta(cluster, kind, app.namespace, app.name, age),
        "spec": {"replicas": replicas, "selector": {"matchLabels": {"app": app.name}}, "template": template(app)},
        "status": {"replicas": replicas, "readyReplicas": ready, "availableReplicas": ready,
            "updatedReplicas": replicas, "currentReplicas": replicas},
    });
    workload["metadata"]["labels"] = json!({"app": app.name});
    if kind == "Deployment" {
        workload["metadata"]["annotations"] = json!({"deployment.kubernetes.io/revision": "7"});
        workload["spec"]["strategy"] = json!({"type": "RollingUpdate"});
    }
    if kind == "DaemonSet" {
        workload["status"] = json!({"desiredNumberScheduled": replicas, "currentNumberScheduled": replicas,
            "numberReady": ready, "numberAvailable": ready, "updatedNumberScheduled": replicas,
            "numberMisscheduled": 0});
    }
    workload
}

/// The conditions of a Deployment whose last rollout was `age` seconds ago: complete, or
/// stuck with pods that are not ready.
pub(super) fn conditions(complete: bool, age: u64) -> Value {
    let (available, progress) = if complete {
        (["True", "MinimumReplicasAvailable"], ["True", "NewReplicaSetAvailable"])
    } else {
        (["False", "MinimumReplicasUnavailable"], ["False", "ProgressDeadlineExceeded"])
    };
    let condition = |kind: &str, [status, reason]: [&str; 2]| {
        let since = ago(age);
        json!({"type": kind, "status": status, "reason": reason, "lastTransitionTime": since})
    };
    json!([condition("Available", available), condition("Progressing", progress)])
}

/// The ReplicaSet of a Deployment: the controller of its pods.
pub(super) fn replica_set(cluster: &str, app: &App, deployment: &Value, hash: &str, pods: (u32, u32)) -> Value {
    let name = format!("{}-{hash}", app.name);
    let mut set = workload(cluster, app, "ReplicaSet", pods, 0);
    set["metadata"] = deployment["metadata"].clone();
    set["metadata"]["name"] = name.as_str().into();
    set["metadata"]["uid"] = uid(&[cluster, "ReplicaSet", app.namespace, &name]).into();
    let owner = deployment["metadata"]["uid"].as_str().unwrap_or_default();
    set["metadata"]["ownerReferences"] = owner_reference("Deployment", app.name, owner);
    set
}

/// The CronJob of an app that runs at 02:00, and its Job of today with one pod that ended.
pub(super) fn cron_job(cluster: &str, app: &App, job_name: &str, age: u64) -> (Value, Value) {
    let cron = json!({
        "metadata": meta(cluster, "CronJob", app.namespace, app.name, 41 * DAY),
        "spec": {"schedule": "0 2 * * *", "successfulJobsHistoryLimit": 1,
            "jobTemplate": {"spec": {"template": template(app)}}},
        "status": {"lastScheduleTime": ago(age), "lastSuccessfulTime": ago(age - 95)},
    });
    let mut job = json!({
        "metadata": meta(cluster, "Job", app.namespace, job_name, age),
        "spec": {"completions": 1, "template": template(app)},
        "status": {"succeeded": 1, "startTime": ago(age), "completionTime": ago(age - 95),
            "conditions": [{"type": "Complete", "status": "True"}]},
    });
    let owner = cron["metadata"]["uid"].as_str().unwrap_or_default();
    job["metadata"]["ownerReferences"] = owner_reference("CronJob", app.name, owner);
    (cron, job)
}

/// A node with 4 cores and 8 GiB, ready, and cordoned when `cordoned` is true.
pub(super) fn node(cluster: &str, name: &str, version: &str, age: u64, cordoned: bool) -> Value {
    let capacity = json!({"cpu": "4", "memory": "8148668Ki", "pods": "110"});
    let allocatable = json!({"cpu": "3900m", "memory": "6919644Ki", "pods": "110"});
    let place = hash(&[cluster, name]);
    let address = format!("10.110.{}.{}", place % 4, 2 + (place >> 8) % 250);
    let condition = |kind: &str, status: &str| json!({"type": kind, "status": status, "lastTransitionTime": ago(age)});
    let mut node = json!({
        "metadata": meta(cluster, "Node", "", name, age),
        "spec": {},
        "status": {
            "capacity": capacity, "allocatable": allocatable,
            "conditions": [condition("MemoryPressure", "False"), condition("DiskPressure", "False"),
                condition("PIDPressure", "False"), condition("Ready", "True")],
            "addresses": [{"type": "InternalIP", "address": address}, {"type": "Hostname", "address": name}],
            "nodeInfo": {"architecture": "amd64", "bootID": "", "containerRuntimeVersion": "containerd://1.7.27",
                "kernelVersion": "6.1.0-31-amd64", "kubeProxyVersion": version, "kubeletVersion": version,
                "machineID": "", "operatingSystem": "linux", "osImage": "Debian GNU/Linux 12 (bookworm)",
                "systemUUID": ""},
        },
    });
    node["metadata"]["labels"] = json!({"kubernetes.io/hostname": name, "kubernetes.io/os": "linux"});
    if cordoned {
        node["spec"] = json!({"unschedulable": true,
            "taints": [{"key": "node.kubernetes.io/unschedulable", "effect": "NoSchedule"}]});
    }
    node
}

/// A warning Event of a pod: its reason and message, its count and the age of the last one.
pub(super) struct Warning<'a> {
    pub(super) reason: &'a str,
    pub(super) message: &'a str,
    pub(super) count: u32,
    pub(super) last: u64,
}

/// The Event of a warning about the pod with this metadata.
pub(super) fn event(cluster: &str, pod: &Value, warning: &Warning<'_>) -> Value {
    let Warning { reason, message, count, last } = *warning;
    let (namespace, name) = (pod["namespace"].as_str().unwrap_or_default(), pod["name"].as_str().unwrap_or_default());
    let event = format!("{name}.{:x}", hash(&[cluster, name, reason]) % 0x1_0000_0000);
    let source = if reason == "FailedScheduling" { "default-scheduler" } else { "kubelet" };
    json!({
        "metadata": meta(cluster, "Event", namespace, &event, last + u64::from(count) * 30),
        "involvedObject": {"kind": "Pod", "namespace": namespace, "name": name, "uid": pod["uid"]},
        "reason": reason, "message": message, "type": "Warning", "count": count,
        "firstTimestamp": ago(last + u64::from(count) * 30), "lastTimestamp": ago(last),
        "source": {"component": source},
    })
}

/// A Service of an app on one port.
pub(super) fn service(cluster: &str, (namespace, name, app): (&str, &str, &str), port: u16) -> Value {
    let address = hash(&[cluster, namespace, name]);
    json!({
        "metadata": meta(cluster, "Service", namespace, name, 61 * DAY),
        "spec": {"type": "ClusterIP", "clusterIP": format!("10.245.{}.{}", address % 64, 2 + address % 250),
            "ports": [{"port": port, "targetPort": port, "protocol": "TCP"}], "selector": {"app": app}},
    })
}
