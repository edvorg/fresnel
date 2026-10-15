//! The sections of the two classes of a pod (`research/01` section 3.4.5, `research/02`
//! section 4.3): a PriorityClass with its value and its preemption, and a RuntimeClass with
//! its handler, the overhead that it adds to each pod and where its pods run.

use egui::{Label, RichText, Ui};
use k8s_openapi::api::node::v1::RuntimeClass;
use k8s_openapi::api::scheduling::v1::PriorityClass;

use crate::format::short;
use crate::views::detail::fields::{ITEM_LIMIT, NONE, VALUE_LIMIT, chips, field, more};
use crate::views::detail::lines::{details_heading, line};
use crate::views::detail::sections::Context;
use crate::views::objects::Link;
use crate::views::pod_spec::texts::{resources, toleration};

pub(crate) fn priority_details(ui: &mut Ui, cx: &Context<'_>, class: &PriorityClass) -> Option<Link> {
    details_heading(ui, cx);
    line(ui, cx, "Value", Some(class.value.to_string()));
    let default = class.global_default == Some(true);
    let default = if default { "yes: a pod with no class gets this one" } else { "no" };
    line(ui, cx, "Global default", Some(default.to_owned()));
    let preemption = class.preemption_policy.clone().unwrap_or_else(|| "PreemptLowerPriority".to_owned());
    line(ui, cx, "Preemption", Some(preemption));
    line(ui, cx, "Description", class.description.clone().filter(|text| !text.is_empty()));
    ui.add_space(10.0);
    None
}

pub(crate) fn runtime_details(ui: &mut Ui, cx: &Context<'_>, class: &RuntimeClass) -> Option<Link> {
    details_heading(ui, cx);
    line(ui, cx, "Handler", Some(class.handler.clone()));
    let overhead = class.overhead.as_ref().and_then(|overhead| resources(overhead.pod_fixed.as_ref()));
    line(ui, cx, "Pod overhead", overhead);
    let scheduling = class.scheduling.as_ref();
    field(ui, cx.palette, "Node selector", |ui| {
        chips(ui, cx.palette, scheduling.and_then(|scheduling| scheduling.node_selector.as_ref()));
    });
    let tolerations = scheduling.and_then(|scheduling| scheduling.tolerations.as_deref()).unwrap_or_default();
    field(ui, cx.palette, "Tolerations", |ui| {
        if tolerations.is_empty() {
            ui.label(RichText::new(NONE).color(cx.palette.text_faint));
        }
        for each in tolerations.iter().take(ITEM_LIMIT) {
            ui.add(Label::new(short(&toleration(each), VALUE_LIMIT)).wrap());
        }
        more(ui, cx.palette, tolerations.len());
    });
    ui.add_space(10.0);
    None
}
