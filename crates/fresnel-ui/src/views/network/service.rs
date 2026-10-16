//! The sections of a Service (`research/01` section 3.4.3, `research/02` section 4.4): its
//! type, its addresses inside and outside the cluster, its selector, its policies and its
//! ports, each TCP port with its Forward button; then its endpoints, with the pod and the
//! node of each one as links.
//!
//! The endpoints come from the owner index, from the same read as the pods behind the
//! Service, so they have the same basis.

use std::collections::BTreeMap;

use egui::Ui;
use k8s_openapi::api::core::v1::{LoadBalancerIngress, Service, ServiceSpec};
use k8s_openapi::api::discovery::v1::EndpointSlice;

use super::endpoints::slice_endpoints;
use super::port;
use crate::views::detail::fields::{NONE, heading, note};
use crate::views::detail::lines::{details_heading, line, none, number, table};
use crate::views::detail::related::basis_note;
use crate::views::detail::sections::{Context, Pick};
use crate::views::objects::Link;
use crate::views::pod_spec::forward_button;

pub(crate) fn details(ui: &mut Ui, cx: &Context<'_>, service: &Service) -> Option<Pick> {
    let spec = service.spec.as_ref();
    let kind = spec.and_then(|spec| spec.type_.as_deref()).unwrap_or("ClusterIP");
    details_heading(ui, cx);
    line(ui, cx, "Type", Some(kind.to_owned()));
    if kind == "ExternalName" {
        line(ui, cx, "External name", spec.and_then(|spec| spec.external_name.clone()));
    } else {
        line(ui, cx, "Cluster IPs", spec.map(cluster_ips));
        line(ui, cx, "IP families", spec.map(families));
    }
    if matches!(kind, "LoadBalancer" | "NodePort") || has_external_ips(spec) {
        line(ui, cx, "External", Some(external(service, kind == "LoadBalancer")));
    }
    let selector = spec.and_then(|spec| spec.selector.as_ref()).filter(|selector| !selector.is_empty());
    let selector = selector.map_or_else(|| "none: the endpoints come from outside".to_owned(), labels);
    line(ui, cx, "Selector", Some(selector));
    line(ui, cx, "Session affinity", spec.map(affinity));
    let policies = spec.map(|spec| {
        let internal = spec.internal_traffic_policy.as_deref().unwrap_or("Cluster");
        match spec.external_traffic_policy.as_deref() {
            Some(external) => format!("external {external}, internal {internal}"),
            None => format!("internal {internal}"),
        }
    });
    line(ui, cx, "Traffic policy", policies);
    let optional = [
        ("Traffic distribution", spec.and_then(|spec| spec.traffic_distribution.clone())),
        ("Balancer class", spec.and_then(|spec| spec.load_balancer_class.clone())),
        (
            "Source ranges",
            spec.and_then(|spec| spec.load_balancer_source_ranges.as_ref()).map(|ranges| ranges.join(", ")),
        ),
        ("Health check port", spec.and_then(|spec| spec.health_check_node_port).map(|port| port.to_string())),
        (
            "Not ready",
            (spec.and_then(|spec| spec.publish_not_ready_addresses) == Some(true)).then(|| "published".to_owned()),
        ),
    ];
    for (label, value) in optional {
        if value.as_deref().is_some_and(|value| !value.is_empty()) {
            line(ui, cx, label, value);
        }
    }
    ui.add_space(10.0);
    heading(ui, cx.palette, "PORTS");
    let ports = spec.and_then(|spec| spec.ports.as_deref()).unwrap_or_default();
    if ports.is_empty() {
        none(ui, cx);
    }
    let mut picked = None;
    table(ui, cx, "service ports", &["Port", "Target", "Node port", "App protocol", ""], ports, |ui, each| {
        ui.label(port(each.name.as_deref(), Some(each.port), each.protocol.as_deref()));
        ui.label(each.target_port.as_ref().map_or_else(|| each.port.to_string(), number));
        ui.label(each.node_port.map_or_else(|| NONE.to_owned(), |port| port.to_string()));
        ui.label(each.app_protocol.as_deref().unwrap_or(NONE));
        let tcp = each.protocol.as_deref().is_none_or(|protocol| protocol == "TCP");
        match u16::try_from(each.port).ok().filter(|_| tcp) {
            Some(number) if forward_button(ui, &format!("Forward port {number} of the Service")) => {
                picked = Some(Pick::Forward(number));
            }
            Some(_) => {}
            None => {
                ui.label("");
            }
        }
    });
    ui.add_space(10.0);
    picked
}

/// The endpoints of the EndpointSlices of the Service.
pub(crate) fn parts(ui: &mut Ui, cx: &Context<'_>) -> Option<Link> {
    heading(ui, cx.palette, "ENDPOINTS");
    let Some(data) = cx.data else {
        note(ui, cx.palette.warn, "The endpoints are not loaded: the cluster is not connected.");
        ui.add_space(10.0);
        return None;
    };
    let answer = data.owners.endpoint_slices_of(&cx.id.key);
    if let Some(text) = basis_note("endpoints", "owner index", &answer.basis) {
        note(ui, cx.palette.warn, &text);
    }
    let slices: Vec<&EndpointSlice> = answer.found.iter().map(|slice| &**slice).collect();
    if slices.iter().all(|slice| slice.endpoints.is_empty()) && answer.basis.is_loaded() {
        none(ui, cx);
    }
    let clicked = slice_endpoints(ui, cx, &slices);
    ui.add_space(10.0);
    clicked
}

/// "10.43.0.10" or "None (headless)": a Service with no cluster IP has a DNS name for each pod.
fn cluster_ips(spec: &ServiceSpec) -> String {
    let ips = spec.cluster_ips.clone().filter(|ips| !ips.is_empty());
    let ips = ips.or_else(|| spec.cluster_ip.clone().map(|ip| vec![ip])).unwrap_or_default();
    match ips.as_slice() {
        [] => NONE.to_owned(),
        [one] if one == "None" => "None (headless)".to_owned(),
        all => all.join(", "),
    }
}

/// "IPv4 (SingleStack)".
fn families(spec: &ServiceSpec) -> String {
    let families = spec.ip_families.as_deref().unwrap_or_default().join(", ");
    let policy = spec.ip_family_policy.as_deref().unwrap_or("SingleStack");
    if families.is_empty() { policy.to_owned() } else { format!("{families} ({policy})") }
}

fn has_external_ips(spec: Option<&ServiceSpec>) -> bool {
    spec.and_then(|spec| spec.external_ips.as_ref()).is_some_and(|ips| !ips.is_empty())
}

/// The addresses outside the cluster: of the load balancer, with its ports, and the external
/// IPs. A load balancer with no address yet is "pending".
fn external(service: &Service, balancer: bool) -> String {
    let entries = service.status.as_ref().and_then(|status| status.load_balancer.as_ref());
    let entries = entries.and_then(|balancer| balancer.ingress.as_deref()).unwrap_or_default();
    let ips = service.spec.as_ref().and_then(|spec| spec.external_ips.as_deref()).unwrap_or_default();
    let mut all: Vec<String> = entries.iter().map(entry).collect();
    all.extend(ips.iter().cloned());
    match all.is_empty() {
        true if balancer => "pending: the load balancer has no address yet".to_owned(),
        true => NONE.to_owned(),
        false => all.join(", "),
    }
}

/// One address of a load balancer, with the error of a port that it gives.
fn entry(entry: &LoadBalancerIngress) -> String {
    let address = entry.ip.as_deref().filter(|ip| !ip.is_empty()).or(entry.hostname.as_deref()).unwrap_or(NONE);
    let errors = entry.ports.iter().flatten().filter_map(|port| {
        Some(format!("{}/{}: {}", port.port, port.protocol, port.error.as_deref().filter(|error| !error.is_empty())?))
    });
    let errors: Vec<String> = errors.collect();
    if errors.is_empty() { address.to_owned() } else { format!("{address} ({})", errors.join(", ")) }
}

/// "ClientIP (for 10800 s)" or "None".
fn affinity(spec: &ServiceSpec) -> String {
    let kind = spec.session_affinity.as_deref().unwrap_or("None");
    let config = spec.session_affinity_config.as_ref().and_then(|config| config.client_ip.as_ref());
    match config.and_then(|config| config.timeout_seconds) {
        Some(seconds) if kind == "ClientIP" => format!("{kind} (for {seconds} s)"),
        _ => kind.to_owned(),
    }
}

/// The labels of a map as `key=value`, in the sequence of the keys.
fn labels(map: &BTreeMap<String, String>) -> String {
    map.iter().map(|(key, value)| format!("{key}={value}")).collect::<Vec<_>>().join(",")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn service(value: serde_json::Value) -> Service {
        serde_json::from_value(value).expect("a Service")
    }

    #[test]
    fn the_addresses_of_a_service_read_as_kubectl_describe_tells_them() {
        let headless = service(json!({"spec": {"clusterIP": "None", "clusterIPs": ["None"]}}));
        assert_eq!(headless.spec.as_ref().map(cluster_ips).as_deref(), Some("None (headless)"));
        let dual = service(json!({"spec": {"clusterIPs": ["10.43.0.9", "fd00::9"], "ipFamilies": ["IPv4", "IPv6"],
            "ipFamilyPolicy": "PreferDualStack"}}));
        let spec = dual.spec.as_ref().expect("a spec");
        assert_eq!(
            (cluster_ips(spec), families(spec)),
            ("10.43.0.9, fd00::9".to_owned(), "IPv4, IPv6 (PreferDualStack)".to_owned())
        );
        let pending = service(json!({"spec": {"type": "LoadBalancer"}}));
        assert_eq!(external(&pending, true), "pending: the load balancer has no address yet");
        let balanced = service(json!({"spec": {"type": "LoadBalancer", "externalIPs": ["192.0.2.9"]},
            "status": {"loadBalancer": {"ingress": [{"ip": "192.0.2.7",
                "ports": [{"port": 443, "protocol": "TCP", "error": "Pending"}]}, {"hostname": "lb.example.com"}]}}}));
        assert_eq!(external(&balanced, true), "192.0.2.7 (443/TCP: Pending), lb.example.com, 192.0.2.9");
        let sticky = service(json!({"spec": {"sessionAffinity": "ClientIP",
            "sessionAffinityConfig": {"clientIP": {"timeoutSeconds": 600}}}}));
        assert_eq!(sticky.spec.as_ref().map(affinity).as_deref(), Some("ClientIP (for 600 s)"));
    }
}
