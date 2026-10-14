//! The sections of the detail panel of a pod (plan section 11.4): its status text in the
//! header, and its node in the metadata. M3 adds the containers and the volumes.

use egui::Ui;
use fresnel_core::kinds::{ObjectHealth, PodStatus};
use k8s_openapi::api::core::v1::Pod;

use crate::theme::Palette;
use crate::views::detail::fields::{NONE, text_field};

/// The status text of `kubectl get pods` with its health value.
pub(crate) fn state(pod: &Pod) -> (String, ObjectHealth) {
    let status = PodStatus::of(pod);
    let text = if status.text.is_empty() { NONE.to_owned() } else { status.text };
    (text, status.health)
}

pub(crate) fn fields(ui: &mut Ui, palette: &Palette, pod: &Pod) {
    let node = pod.spec.as_ref().and_then(|spec| spec.node_name.as_deref());
    text_field(ui, palette, "Node", node.unwrap_or(NONE));
}
