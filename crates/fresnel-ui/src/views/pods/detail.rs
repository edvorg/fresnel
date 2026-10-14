//! The sections of the detail panel of a pod (plan section 11.4, `research/01` section 3.4.2
//! and `research/02` section 4.2): its status text by the kubectl rule in the header, its
//! details, and the containers and the volumes of its spec.

use egui::Ui;
use fresnel_core::kinds::{Kind, ObjectHealth, PodStatus};
use fresnel_core::store::ObjectKey;
use k8s_openapi::api::core::v1::Pod;

use crate::views::detail::fields::{NONE, field, heading, text_field};
use crate::views::detail::sections::Context;
use crate::views::objects::Link;
use crate::views::pod_spec::{self, scheduling, texts};

/// The status text of `kubectl get pods` with its health value.
pub(crate) fn state(pod: &Pod) -> (String, ObjectHealth) {
    let status = PodStatus::of(pod);
    let text = if status.text.is_empty() { NONE.to_owned() } else { status.text };
    (text, status.health)
}

/// The details: the node, the addresses, the class of service, the priority, the times and
/// what the pod runs as. Gives the object of a link that the user clicked.
pub(crate) fn details(ui: &mut Ui, cx: &Context<'_>, pod: &Pod) -> Option<Link> {
    let (palette, clock) = (cx.palette, cx.clock);
    let (spec, status) = (pod.spec.as_ref(), pod.status.as_ref());
    heading(ui, palette, "DETAILS");
    let mut clicked = None;
    let node = spec.and_then(|spec| spec.node_name.as_deref()).filter(|node| !node.is_empty());
    field(ui, palette, "Node", |ui| match node {
        Some(node) => {
            if ui.link(node).clicked() {
                let key = ObjectKey::new("", node);
                clicked = Some(Link { cluster: cx.id.cluster.clone(), kind: Kind::Node, key });
            }
        }
        None => {
            ui.label(NONE);
        }
    });
    let addresses = |list: Option<Vec<String>>, one: Option<&String>| {
        let list = list.filter(|list| !list.is_empty()).or_else(|| one.map(|one| vec![one.clone()]));
        list.map_or_else(|| NONE.to_owned(), |list| list.join(", "))
    };
    let pod_ips = status.and_then(|status| status.pod_ips.as_ref()).map(|ips| ips.iter().map(|ip| ip.ip.clone()));
    let host_ips = status.and_then(|status| status.host_ips.as_ref()).map(|ips| ips.iter().map(|ip| ip.ip.clone()));
    text_field(
        ui,
        palette,
        "Pod IPs",
        &addresses(pod_ips.map(Iterator::collect), status.and_then(|s| s.pod_ip.as_ref())),
    );
    let host_ip = status.and_then(|status| status.host_ip.as_ref());
    text_field(ui, palette, "Host IPs", &addresses(host_ips.map(Iterator::collect), host_ip));
    let line = |ui: &mut Ui, label: &str, value: Option<String>| {
        text_field(ui, palette, label, value.as_deref().unwrap_or(NONE));
    };
    line(ui, "QoS class", status.and_then(|status| status.qos_class.clone()));
    line(ui, "Priority", spec.and_then(|spec| spec.priority).map(|priority| priority.to_string()));
    line(ui, "Started", status.and_then(|status| status.start_time.as_ref()).map(|time| texts::at(time, clock)));
    let grace = spec.and_then(|spec| spec.termination_grace_period_seconds);
    line(ui, "Grace period", Some(format!("{} s", grace.unwrap_or(30))));
    line(ui, "Restart policy", spec.and_then(|spec| spec.restart_policy.clone()));
    let nominated = status.and_then(|status| status.nominated_node_name.clone());
    if nominated.is_some() {
        line(ui, "Nominated node", nominated);
    }
    for (label, value) in
        [("Reason", status.and_then(|s| s.reason.clone())), ("Message", status.and_then(|s| s.message.clone()))]
    {
        if value.as_deref().is_some_and(|value| !value.is_empty()) {
            line(ui, label, value);
        }
    }
    if let Some(spec) = spec {
        clicked = scheduling::show(ui, &cx.place(), spec).or(clicked);
    }
    ui.add_space(10.0);
    clicked
}

/// The containers, with the status of each one, and the volumes.
pub(crate) fn parts(ui: &mut Ui, cx: &Context<'_>, pod: &Pod) -> Option<Link> {
    let spec = pod.spec.as_ref()?;
    // A pod with no status yet shows each container with no state, not as a template.
    let none = k8s_openapi::api::core::v1::PodStatus::default();
    pod_spec::show(ui, &cx.place(), spec, Some(pod.status.as_ref().unwrap_or(&none)))
}
