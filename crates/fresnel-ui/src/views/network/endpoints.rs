//! The endpoints of a Service in their two forms: an EndpointSlice, and the older Endpoints
//! (`research/01` section 3.4.3). Each address has its state, its pod as a link, its node
//! and its zone; then the ports. The view of a Service shows the endpoints of its slices.

use egui::{RichText, Ui};
use fresnel_core::kinds::Kind;
use k8s_openapi::api::core::v1::{EndpointAddress, Endpoints, ObjectReference};
use k8s_openapi::api::discovery::v1::{Endpoint, EndpointConditions, EndpointSlice};

use super::port;
use crate::views::detail::fields::{NONE, field, heading};
use crate::views::detail::lines::{details_heading, line, none, table};
use crate::views::detail::sections::Context;
use crate::views::objects::Link;
use crate::widgets::dot;
use crate::widgets::tone::Tone;

/// The label of a slice with the name of its Service.
const SERVICE_LABEL: &str = "kubernetes.io/service-name";
/// The label of a slice with the controller that writes it.
const MANAGER_LABEL: &str = "endpointslice.kubernetes.io/managed-by";

/// The address type, the Service and the controller of a slice, its endpoints and its ports.
pub(crate) fn slice_details(ui: &mut Ui, cx: &Context<'_>, slice: &EndpointSlice) -> Option<Link> {
    details_heading(ui, cx);
    line(ui, cx, "Address type", Some(slice.address_type.clone()));
    let labels = slice.metadata.labels.as_ref();
    let mut clicked = None;
    field(ui, cx.palette, "Service", |ui| match labels.and_then(|labels| labels.get(SERVICE_LABEL)) {
        Some(name) => clicked = cx.link(ui, name, Kind::Service, &cx.id.key.namespace, name),
        None => {
            ui.label(NONE);
        }
    });
    line(ui, cx, "Managed by", labels.and_then(|labels| labels.get(MANAGER_LABEL)).cloned());
    ui.add_space(10.0);
    heading(ui, cx.palette, "ENDPOINTS");
    if slice.endpoints.is_empty() {
        none(ui, cx);
    }
    clicked = slice_endpoints(ui, cx, &[slice]).or(clicked);
    ui.add_space(10.0);
    heading(ui, cx.palette, "PORTS");
    let ports = slice.ports.as_deref().unwrap_or_default();
    if ports.is_empty() {
        none(ui, cx);
    }
    table(ui, cx, "slice ports", &["Port", "App protocol"], ports, |ui, slice_port| {
        ui.label(port(slice_port.name.as_deref(), slice_port.port, slice_port.protocol.as_deref()));
        ui.label(slice_port.app_protocol.as_deref().unwrap_or(NONE));
    });
    ui.add_space(10.0);
    clicked
}

/// The endpoints of some slices in one table. Gives the pod or the node that the user clicked.
pub(crate) fn slice_endpoints(ui: &mut Ui, cx: &Context<'_>, slices: &[&EndpointSlice]) -> Option<Link> {
    let rows: Vec<(&str, &Endpoint)> = slices
        .iter()
        .flat_map(|slice| {
            let namespace = slice.metadata.namespace.as_deref().unwrap_or_default();
            slice.endpoints.iter().map(move |endpoint| (namespace, endpoint))
        })
        .collect();
    let mut clicked = None;
    let titles = ["Addresses", "State", "Pod", "Node", "Zone"];
    table(ui, cx, "endpoints", &titles, &rows, |ui, (namespace, endpoint)| {
        ui.label(endpoint.addresses.join(", "));
        let (text, tone) = state(endpoint.conditions.as_ref());
        ui.horizontal(|ui| {
            dot(ui, tone.color(cx.palette));
            ui.label(RichText::new(text).color(tone.color(cx.palette)));
        });
        clicked = target(ui, cx, endpoint.target_ref.as_ref(), namespace).or(clicked.take());
        clicked = node(ui, cx, endpoint.node_name.as_deref()).or(clicked.take());
        ui.label(RichText::new(endpoint.zone.as_deref().unwrap_or(NONE)).color(cx.palette.text_muted));
    });
    clicked
}

/// The state of an endpoint, as its conditions tell it. With no condition, an endpoint is
/// ready and serves: the rule of the API for a state that is not known.
pub(super) fn state(conditions: Option<&EndpointConditions>) -> (&'static str, Tone) {
    let ready = conditions.and_then(|conditions| conditions.ready).unwrap_or(true);
    let serving = conditions.and_then(|conditions| conditions.serving).unwrap_or(ready);
    let terminating = conditions.and_then(|conditions| conditions.terminating).unwrap_or(false);
    match (terminating, serving, ready) {
        (true, true, _) => ("terminating, serves", Tone::Info),
        (true, false, _) => ("terminating", Tone::Muted),
        (false, _, true) => ("ready", Tone::Ok),
        (false, _, false) => ("not ready", Tone::Warn),
    }
}

/// The addresses of the older Endpoints, each one with its state, its target, its node and
/// the ports of its subset.
pub(crate) fn details(ui: &mut Ui, cx: &Context<'_>, endpoints: &Endpoints) -> Option<Link> {
    let mut rows: Vec<(&EndpointAddress, bool, String)> = Vec::new();
    for subset in endpoints.subsets.iter().flatten() {
        let ports = subset
            .ports
            .iter()
            .flatten()
            .map(|each| port(each.name.as_deref(), Some(each.port), each.protocol.as_deref()));
        let ports = ports.collect::<Vec<_>>().join(", ");
        rows.extend(subset.addresses.iter().flatten().map(|address| (address, true, ports.clone())));
        rows.extend(subset.not_ready_addresses.iter().flatten().map(|address| (address, false, ports.clone())));
    }
    heading(ui, cx.palette, "ADDRESSES");
    if rows.is_empty() {
        none(ui, cx);
    }
    let mut clicked = None;
    let titles = ["Address", "State", "Target", "Node", "Ports"];
    table(ui, cx, "addresses", &titles, &rows, |ui, (address, ready, ports)| {
        ui.label(&address.ip);
        let (text, tone) = if *ready { ("ready", Tone::Ok) } else { ("not ready", Tone::Warn) };
        ui.horizontal(|ui| {
            dot(ui, tone.color(cx.palette));
            ui.label(RichText::new(text).color(tone.color(cx.palette)));
        });
        clicked = target(ui, cx, address.target_ref.as_ref(), &cx.id.key.namespace).or(clicked.take());
        clicked = node(ui, cx, address.node_name.as_deref()).or(clicked.take());
        ui.label(if ports.is_empty() { NONE } else { ports.as_str() });
    });
    ui.add_space(10.0);
    clicked
}

/// The object behind an address, as a link: mostly a pod.
fn target(ui: &mut Ui, cx: &Context<'_>, target: Option<&ObjectReference>, namespace: &str) -> Option<Link> {
    let Some((kind, name)) = target.and_then(|target| Some((target.kind.as_deref()?, target.name.as_deref()?))) else {
        ui.label(NONE);
        return None;
    };
    let namespace = target.and_then(|target| target.namespace.as_deref()).unwrap_or(namespace);
    if kind == "Pod" { cx.link(ui, name, Kind::Pod, namespace, name) } else { cx.reference(ui, kind, namespace, name) }
}

/// The node of an address, as a link.
fn node(ui: &mut Ui, cx: &Context<'_>, node: Option<&str>) -> Option<Link> {
    match node.filter(|node| !node.is_empty()) {
        Some(node) => cx.link(ui, node, Kind::Node, "", node),
        None => {
            ui.label(NONE);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_endpoint_state_follows_the_rule_of_the_api_for_each_condition() {
        let conditions = |ready, serving, terminating| EndpointConditions { ready, serving, terminating };
        assert_eq!(state(None).0, "ready", "no condition: ready");
        assert_eq!(state(Some(&conditions(Some(false), None, None))).0, "not ready");
        assert_eq!(state(Some(&conditions(Some(false), Some(true), Some(true)))).0, "terminating, serves");
        assert_eq!(state(Some(&conditions(Some(false), Some(false), Some(true)))).0, "terminating");
        assert_eq!(state(Some(&conditions(None, None, Some(false)))).0, "ready");
    }
}
