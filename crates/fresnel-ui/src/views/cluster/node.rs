//! The sections of a Node (`research/01` section 3.4.1, `research/02` section 4.5): whether
//! it takes new pods, its addresses, its pod ranges, its taints and its system; then the
//! capacity and the allocatable part of each resource, and the pods on it with their state.
//!
//! The reference index gives the pods on the node, so "no pod" and "the pods are not
//! loaded" never look the same. The labels and the conditions are in the sections of each kind.

use std::collections::BTreeSet;

use egui::{Label, RichText, Ui};
use fresnel_core::kinds::Kind;
use fresnel_core::store::ObjectKey;
use k8s_openapi::api::core::v1::{Node, Taint};

use crate::format::short;
use crate::views::detail::fields::{ITEM_LIMIT, NONE, VALUE_LIMIT, field, heading, more, note};
use crate::views::detail::lines::{details_heading, line, none, table};
use crate::views::detail::sections::Context;
use crate::views::objects::Link;
use crate::views::pod_spec::texts::at;
use crate::views::workloads::pods::{Last, pods};

pub(crate) fn details(ui: &mut Ui, cx: &Context<'_>, node: &Node) -> Option<Link> {
    let (spec, status) = (node.spec.as_ref(), node.status.as_ref());
    details_heading(ui, cx);
    let closed = spec.and_then(|spec| spec.unschedulable) == Some(true);
    let (text, color) = if closed { ("no: the node is cordoned", cx.palette.warn) } else { ("yes", cx.palette.text) };
    field(ui, cx.palette, "New pods", |ui| {
        ui.label(RichText::new(text).color(color));
    });
    let addresses = status.and_then(|status| status.addresses.as_deref()).unwrap_or_default();
    let addresses: Vec<String> =
        addresses.iter().map(|address| format!("{} {}", address.type_, address.address)).collect();
    line(ui, cx, "Addresses", (!addresses.is_empty()).then(|| addresses.join(", ")));
    let ranges = spec.and_then(|spec| {
        spec.pod_cidrs.clone().filter(|ranges| !ranges.is_empty()).or(spec.pod_cidr.clone().map(|range| vec![range]))
    });
    line(ui, cx, "Pod ranges", ranges.map(|ranges| ranges.join(", ")));
    line(ui, cx, "Provider ID", spec.and_then(|spec| spec.provider_id.clone()));
    let taints = spec.and_then(|spec| spec.taints.as_deref()).unwrap_or_default();
    field(ui, cx.palette, "Taints", |ui| {
        if taints.is_empty() {
            ui.label(RichText::new(NONE).color(cx.palette.text_faint));
        }
        for each in taints.iter().take(ITEM_LIMIT) {
            ui.add(Label::new(short(&taint(each, cx), VALUE_LIMIT)).wrap());
        }
        more(ui, cx.palette, taints.len());
    });
    if let Some(info) = status.and_then(|status| status.node_info.as_ref()) {
        line(ui, cx, "OS", Some(format!("{} ({}/{})", info.os_image, info.operating_system, info.architecture)));
        line(ui, cx, "Kernel", Some(info.kernel_version.clone()));
        line(ui, cx, "Runtime", Some(info.container_runtime_version.clone()));
        line(ui, cx, "Kubelet", Some(info.kubelet_version.clone()));
    }
    ui.add_space(10.0);
    heading(ui, cx.palette, "CAPACITY");
    let capacity = status.and_then(|status| status.capacity.as_ref());
    let allocatable = status.and_then(|status| status.allocatable.as_ref());
    let names: BTreeSet<&String> = capacity.into_iter().chain(allocatable).flat_map(|map| map.keys()).collect();
    let names: Vec<&String> = names.into_iter().collect();
    if names.is_empty() {
        none(ui, cx);
    }
    table(ui, cx, "capacity", &["Resource", "Capacity", "Allocatable"], &names, |ui, name| {
        ui.label(name.as_str());
        for map in [capacity, allocatable] {
            ui.label(map.and_then(|map| map.get(*name)).map_or(NONE, |quantity| quantity.0.as_str()));
        }
    });
    ui.add_space(10.0);
    None
}

/// The pods on the node, with their namespace.
pub(crate) fn parts(ui: &mut Ui, cx: &Context<'_>) -> Option<Link> {
    heading(ui, cx.palette, "PODS");
    let Some(data) = cx.data else {
        note(ui, cx.palette.warn, "The pods are not loaded: the cluster is not connected.");
        ui.add_space(10.0);
        return None;
    };
    let answer = data.references.used_by(Kind::Node, &cx.id.key);
    let keys: Vec<&ObjectKey> = answer.found.iter().collect();
    let clicked = pods(ui, cx, data, (&keys, &answer.basis, "reference index"), Last::Namespace);
    ui.add_space(10.0);
    clicked
}

/// A taint as kubectl prints it, with the time of a taint that the node controller added:
/// `node.kubernetes.io/unreachable:NoExecute since 2027-01-15 07:58:00 (2 min ago)`.
fn taint(taint: &Taint, cx: &Context<'_>) -> String {
    let value = taint.value.as_deref().filter(|value| !value.is_empty()).map(|value| format!("={value}"));
    let since = taint.time_added.as_ref().map(|time| format!(" since {}", at(time, cx.clock)));
    format!("{}{}:{}{}", taint.key, value.unwrap_or_default(), taint.effect, since.unwrap_or_default())
}
