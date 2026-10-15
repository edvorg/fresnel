//! The texts of the parts of a pod spec, in the words of `kubectl describe`: a probe, the
//! resources, a port, a mount, the state of a container and the source of a volume.

use std::collections::BTreeMap;
use std::time::SystemTime;

use fresnel_core::kinds::Kind;
use k8s_openapi::api::core::v1::{ContainerPort, ContainerState, EnvVar, Probe, Toleration, Volume, VolumeMount};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;

use crate::format::{self, Clock};
use crate::widgets::tone::Tone;

/// A probe as `kubectl describe` prints it, with the defaults of the server for each
/// number that the spec does not give: `http-get http://:8080/healthz delay=0s timeout=1s
/// period=10s #success=1 #failure=3`.
pub(super) fn probe(probe: &Probe) -> String {
    let port = |port: &IntOrString| match port {
        IntOrString::Int(number) => number.to_string(),
        IntOrString::String(name) => name.clone(),
    };
    let action = if let Some(get) = &probe.http_get {
        let scheme = get.scheme.as_deref().unwrap_or("HTTP").to_lowercase();
        let host = get.host.as_deref().unwrap_or_default();
        format!("http-get {scheme}://{host}:{}{}", port(&get.port), get.path.as_deref().unwrap_or_default())
    } else if let Some(tcp) = &probe.tcp_socket {
        format!("tcp-socket {}:{}", tcp.host.as_deref().unwrap_or_default(), port(&tcp.port))
    } else if let Some(exec) = &probe.exec {
        format!("exec [{}]", exec.command.as_deref().unwrap_or_default().join(" "))
    } else if let Some(grpc) = &probe.grpc {
        format!("grpc <pod>:{} {}", grpc.port, grpc.service.as_deref().unwrap_or_default()).trim_end().to_owned()
    } else {
        "unknown".to_owned()
    };
    let number = |value: Option<i32>, default: i32| value.unwrap_or(default);
    format!(
        "{action} delay={}s timeout={}s period={}s #success={} #failure={}",
        number(probe.initial_delay_seconds, 0),
        number(probe.timeout_seconds, 1),
        number(probe.period_seconds, 10),
        number(probe.success_threshold, 1),
        number(probe.failure_threshold, 3),
    )
}

/// "cpu 100m, memory 64Mi". `None` for no value.
pub(crate) fn resources(values: Option<&BTreeMap<String, Quantity>>) -> Option<String> {
    let values = values.filter(|values| !values.is_empty())?;
    Some(values.iter().map(|(name, quantity)| format!("{name} {}", quantity.0)).collect::<Vec<_>>().join(", "))
}

/// "http 8080/TCP", with the port of the node when the spec has one.
pub(super) fn port(port: &ContainerPort) -> String {
    let protocol = port.protocol.as_deref().unwrap_or("TCP");
    let name = port.name.as_deref().map(|name| format!("{name} ")).unwrap_or_default();
    let host = port.host_port.map(|host| format!(", node port {host}")).unwrap_or_default();
    format!("{name}{}/{protocol}{host}", port.container_port)
}

/// "/etc/config from config (ro)", with its sub path.
pub(super) fn mount(mount: &VolumeMount) -> String {
    let sub = mount.sub_path.as_deref().or(mount.sub_path_expr.as_deref()).filter(|path| !path.is_empty());
    let sub = sub.map(|path| format!(", path {path}")).unwrap_or_default();
    let access = if mount.read_only == Some(true) { "ro" } else { "rw" };
    format!("{} from {} ({access}{sub})", mount.mount_path, mount.name)
}

/// The state of a container with the tone of its health. A container with no status has
/// none yet: the kubelet did not report it.
pub(super) fn state(state: Option<&ContainerState>, ready: bool, clock: &Clock) -> (String, Tone) {
    let Some(state) = state else {
        return ("No state yet".to_owned(), Tone::Faint);
    };
    if let Some(running) = &state.running {
        let since = running.started_at.as_ref().map(|time| format!(" since {}", at(time, clock))).unwrap_or_default();
        return match ready {
            true => (format!("Running{since}"), Tone::Ok),
            false => (format!("Running, not ready{since}"), Tone::Warn),
        };
    }
    if let Some(waiting) = &state.waiting {
        let reason = waiting.reason.as_deref().unwrap_or("Waiting");
        let tone = if reason.contains("BackOff") || reason.contains("Err") { Tone::Error } else { Tone::Warn };
        return (with_message(format!("Waiting: {reason}"), waiting.message.as_deref()), tone);
    }
    match terminated(state, clock) {
        Some((text, exit_code)) => (text, if exit_code == 0 { Tone::Ok } else { Tone::Error }),
        None => ("No state yet".to_owned(), Tone::Faint),
    }
}

/// The last state of a container that started again: why its run before ended.
pub(super) fn last_state(state: Option<&ContainerState>, clock: &Clock) -> Option<String> {
    terminated(state?, clock).map(|(text, _)| text)
}

/// "Terminated: Error (exit code 1), finished 2027-01-15 07:58:00 (1 min ago)".
fn terminated(state: &ContainerState, clock: &Clock) -> Option<(String, i32)> {
    let ended = state.terminated.as_ref()?;
    let reason = ended.reason.as_deref().unwrap_or("Terminated");
    let signal = ended.signal.map(|signal| format!(", signal {signal}")).unwrap_or_default();
    let finished = ended.finished_at.as_ref().map(|time| format!(", finished {}", at(time, clock))).unwrap_or_default();
    let text = format!("Terminated: {reason} (exit code {}{signal}){finished}", ended.exit_code);
    Some((with_message(text, ended.message.as_deref()), ended.exit_code))
}

fn with_message(text: String, message: Option<&str>) -> String {
    match message.filter(|message| !message.is_empty()) {
        Some(message) => format!("{text}: {message}"),
        None => text,
    }
}

/// A time of the cluster with its age: "2027-01-15 07:59:09 (51 s ago)".
pub(crate) fn at(time: &Time, clock: &Clock) -> String {
    moment(SystemTime::from(time.0), clock)
}

/// A time with its age, of any type of time of the API.
pub(crate) fn moment(time: SystemTime, clock: &Clock) -> String {
    format!("{} ({} ago)", clock.date_time(time), format::span(clock.since(time)))
}

/// A toleration as `kubectl describe` prints it: `node.kubernetes.io/not-ready:NoExecute
/// op=Exists for 300s`.
pub(crate) fn toleration(toleration: &Toleration) -> String {
    let key = toleration.key.as_deref().unwrap_or_default();
    let value = toleration.value.as_deref().filter(|value| !value.is_empty());
    let mut text = match value {
        Some(value) => format!("{key}={value}"),
        None => key.to_owned(),
    };
    if let Some(effect) = toleration.effect.as_deref().filter(|effect| !effect.is_empty()) {
        text.push_str(&format!(":{effect}"));
    }
    if toleration.operator.as_deref() == Some("Exists") {
        text.push_str(" op=Exists");
    }
    if let Some(seconds) = toleration.toleration_seconds {
        text.push_str(&format!(" for {seconds}s"));
    }
    if text.trim().is_empty() { "each taint".to_owned() } else { text.trim().to_owned() }
}

/// The value of an environment variable.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum EnvValue<'a> {
    /// A value of the spec itself.
    Text(&'a str),
    /// A key of a ConfigMap or a Secret: the view links the object and never shows a value
    /// of a Secret, which the spec does not hold.
    Key { kind: Kind, name: &'a str, key: &'a str },
    /// A field of the pod or a resource of a container, as the text of the spec.
    Field(String),
}

pub(super) fn env(var: &EnvVar) -> EnvValue<'_> {
    let Some(from) = &var.value_from else {
        return EnvValue::Text(var.value.as_deref().unwrap_or_default());
    };
    if let Some(selector) = &from.config_map_key_ref {
        return EnvValue::Key { kind: Kind::ConfigMap, name: &selector.name, key: &selector.key };
    }
    if let Some(selector) = &from.secret_key_ref {
        return EnvValue::Key { kind: Kind::Secret, name: &selector.name, key: &selector.key };
    }
    if let Some(field) = &from.field_ref {
        return EnvValue::Field(format!("field {}", field.field_path));
    }
    if let Some(resource) = &from.resource_field_ref {
        let container = resource.container_name.as_deref().map(|name| format!(" of {name}")).unwrap_or_default();
        return EnvValue::Field(format!("resource {}{container}", resource.resource));
    }
    EnvValue::Field("a source of a newer server".to_owned())
}

/// One part of the source of a volume: an object of the cluster or a text.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Part<'a> {
    Object(Kind, &'a str),
    Text(String),
}

/// The type of the source of a volume and its parts.
pub(super) fn volume(volume: &Volume) -> (&'static str, Vec<Part<'_>>) {
    let read_only = |on: Option<bool>| if on == Some(true) { " (ro)" } else { "" };
    if let Some(source) = &volume.config_map {
        return ("ConfigMap", vec![Part::Object(Kind::ConfigMap, &source.name)]);
    }
    if let Some(source) = &volume.secret {
        let name = source.secret_name.as_deref().unwrap_or_default();
        return ("Secret", vec![Part::Object(Kind::Secret, name)]);
    }
    if let Some(claim) = &volume.persistent_volume_claim {
        let text = format!("claim {}{}", claim.claim_name, read_only(claim.read_only));
        return ("PersistentVolumeClaim", vec![Part::Text(text)]);
    }
    if let Some(empty) = &volume.empty_dir {
        let medium = empty.medium.as_deref().filter(|medium| !medium.is_empty()).unwrap_or("disk of the node");
        let limit = empty.size_limit.as_ref().map(|limit| format!(", limit {}", limit.0)).unwrap_or_default();
        return ("EmptyDir", vec![Part::Text(format!("{medium}{limit}"))]);
    }
    if let Some(host) = &volume.host_path {
        let kind = host.type_.as_deref().filter(|kind| !kind.is_empty());
        let text = kind.map_or_else(|| host.path.clone(), |kind| format!("{} ({kind})", host.path));
        return ("HostPath", vec![Part::Text(text)]);
    }
    if let Some(projected) = &volume.projected {
        let parts = projected.sources.iter().flatten().map(|source| {
            if let Some(map) = &source.config_map {
                return Part::Object(Kind::ConfigMap, &map.name);
            }
            if let Some(secret) = &source.secret {
                return Part::Object(Kind::Secret, &secret.name);
            }
            let text = if source.service_account_token.is_some() {
                "token of the ServiceAccount"
            } else if source.downward_api.is_some() {
                "fields of the pod"
            } else {
                "a source of a newer server"
            };
            Part::Text(text.to_owned())
        });
        return ("Projected", parts.collect());
    }
    if volume.downward_api.is_some() {
        return ("DownwardAPI", vec![Part::Text("fields of the pod".to_owned())]);
    }
    if let Some(csi) = &volume.csi {
        return ("CSI", vec![Part::Text(format!("driver {}", csi.driver))]);
    }
    if let Some(nfs) = &volume.nfs {
        return ("NFS", vec![Part::Text(format!("{}:{}", nfs.server, nfs.path))]);
    }
    if let Some(image) = &volume.image {
        return ("Image", vec![Part::Text(image.reference.clone().unwrap_or_default())]);
    }
    if volume.ephemeral.is_some() {
        return ("Ephemeral", vec![Part::Text("a claim of the pod".to_owned())]);
    }
    ("Other", vec![Part::Text("a source that the view does not show".to_owned())])
}

#[cfg(test)]
mod tests {
    use k8s_openapi::api::core::v1::Container;
    use serde_json::json;

    use super::*;

    fn container(value: serde_json::Value) -> Container {
        serde_json::from_value(value).expect("a container")
    }

    #[test]
    fn a_probe_has_the_words_and_the_defaults_of_kubectl_describe() {
        let web = container(json!({"name": "web", "livenessProbe": {"httpGet": {"path": "/healthz", "port": 8080}},
            "readinessProbe": {"tcpSocket": {"port": "http"}, "periodSeconds": 5, "failureThreshold": 1},
            "startupProbe": {"exec": {"command": ["cat", "/ready"]}, "initialDelaySeconds": 3}}));
        let text = |probe: Option<&Probe>| probe.map(super::probe).expect("a probe");
        assert_eq!(
            text(web.liveness_probe.as_ref()),
            "http-get http://:8080/healthz delay=0s timeout=1s period=10s #success=1 #failure=3"
        );
        assert_eq!(
            text(web.readiness_probe.as_ref()),
            "tcp-socket :http delay=0s timeout=1s period=5s #success=1 #failure=1"
        );
        assert_eq!(
            text(web.startup_probe.as_ref()),
            "exec [cat /ready] delay=3s timeout=1s period=10s #success=1 #failure=3"
        );
    }

    #[test]
    fn ports_mounts_and_resources_read_as_short_texts() {
        let web = container(json!({"name": "web",
            "ports": [{"name": "http", "containerPort": 8080}, {"containerPort": 53, "protocol": "UDP", "hostPort": 53}],
            "volumeMounts": [{"name": "config", "mountPath": "/etc/web", "readOnly": true, "subPath": "web.conf"}],
            "resources": {"requests": {"cpu": "100m", "memory": "64Mi"}}}));
        let ports: Vec<String> = web.ports.iter().flatten().map(port).collect();
        assert_eq!(ports, ["http 8080/TCP", "53/UDP, node port 53"]);
        let mounts: Vec<String> = web.volume_mounts.iter().flatten().map(mount).collect();
        assert_eq!(mounts, ["/etc/web from config (ro, path web.conf)"]);
        let requirements = web.resources.expect("resources");
        assert_eq!(resources(requirements.requests.as_ref()).as_deref(), Some("cpu 100m, memory 64Mi"));
        assert_eq!(resources(requirements.limits.as_ref()), None);
    }

    #[test]
    fn a_key_of_a_secret_names_the_secret_and_the_key_and_no_value() {
        let web = container(json!({"name": "web", "env": [
            {"name": "MODE", "value": "fast"},
            {"name": "TOKEN", "valueFrom": {"secretKeyRef": {"name": "web-secret", "key": "token"}}},
            {"name": "LEVEL", "valueFrom": {"configMapKeyRef": {"name": "web-config", "key": "level"}}},
            {"name": "POD", "valueFrom": {"fieldRef": {"fieldPath": "metadata.name"}}}]}));
        let values: Vec<EnvValue<'_>> = web.env.iter().flatten().map(env).collect();
        assert_eq!(
            values,
            [
                EnvValue::Text("fast"),
                EnvValue::Key { kind: Kind::Secret, name: "web-secret", key: "token" },
                EnvValue::Key { kind: Kind::ConfigMap, name: "web-config", key: "level" },
                EnvValue::Field("field metadata.name".to_owned()),
            ]
        );
    }

    #[test]
    fn a_toleration_has_the_form_of_kubectl_describe() {
        let tolerations: Vec<Toleration> = serde_json::from_value(json!([
            {"key": "node.kubernetes.io/not-ready", "operator": "Exists", "effect": "NoExecute",
             "tolerationSeconds": 300},
            {"key": "pool", "operator": "Equal", "value": "gpu", "effect": "NoSchedule"},
            {"operator": "Exists"},
        ]))
        .expect("tolerations");
        let texts: Vec<String> = tolerations.iter().map(toleration).collect();
        assert_eq!(
            texts,
            ["node.kubernetes.io/not-ready:NoExecute op=Exists for 300s", "pool=gpu:NoSchedule", "op=Exists"]
        );
    }

    #[test]
    fn each_source_of_a_volume_has_its_type_and_its_objects() {
        let volumes: Vec<Volume> = serde_json::from_value(json!([
            {"name": "a", "configMap": {"name": "web-config"}},
            {"name": "b", "secret": {"secretName": "web-secret"}},
            {"name": "c", "persistentVolumeClaim": {"claimName": "data", "readOnly": true}},
            {"name": "d", "emptyDir": {"medium": "Memory", "sizeLimit": "1Gi"}},
            {"name": "e", "projected": {"sources": [{"serviceAccountToken": {"path": "token"}},
                {"configMap": {"name": "kube-root-ca.crt"}}]}},
        ]))
        .expect("volumes");
        let read: Vec<(&str, Vec<Part<'_>>)> = volumes.iter().map(volume).collect();
        assert_eq!(read[0], ("ConfigMap", vec![Part::Object(Kind::ConfigMap, "web-config")]));
        assert_eq!(read[1], ("Secret", vec![Part::Object(Kind::Secret, "web-secret")]));
        assert_eq!(read[2], ("PersistentVolumeClaim", vec![Part::Text("claim data (ro)".to_owned())]));
        assert_eq!(read[3], ("EmptyDir", vec![Part::Text("Memory, limit 1Gi".to_owned())]));
        let projected = vec![
            Part::Text("token of the ServiceAccount".to_owned()),
            Part::Object(Kind::ConfigMap, "kube-root-ca.crt"),
        ];
        assert_eq!(read[4], ("Projected", projected));
    }
}
