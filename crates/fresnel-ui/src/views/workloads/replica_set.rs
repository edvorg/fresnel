//! The sections of a ReplicaSet and of a ReplicationController, the older kind with the same
//! task: the replicas and the selector; then the pods and the template.

use egui::Ui;
use fresnel_core::kinds::Kind;
use k8s_openapi::api::apps::v1::ReplicaSet;
use k8s_openapi::api::core::v1::ReplicationController;

use super::{counts, pods, template};
use crate::views::detail::lines::{details_heading, line, seconds, selector};
use crate::views::detail::sections::{Context, Pick};
use crate::views::objects::Link;

pub(crate) fn details(ui: &mut Ui, cx: &Context<'_>, set: &ReplicaSet) -> Option<Link> {
    let (spec, status) = (set.spec.as_ref(), set.status.as_ref());
    details_heading(ui, cx);
    counts(
        ui,
        cx,
        &[
            ("desired", Some(spec.and_then(|spec| spec.replicas).unwrap_or(1))),
            ("current", status.map(|status| status.replicas)),
            ("ready", status.and_then(|status| status.ready_replicas)),
            ("available", status.and_then(|status| status.available_replicas)),
        ],
    );
    line(ui, cx, "Selector", Some(selector(spec.map(|spec| &spec.selector))));
    line(ui, cx, "Min ready", seconds(spec.and_then(|spec| spec.min_ready_seconds), Some(0)));
    ui.add_space(10.0);
    None
}

pub(crate) fn parts(ui: &mut Ui, cx: &Context<'_>, set: &ReplicaSet) -> Option<Pick> {
    let pod = pods::show(ui, cx, Kind::ReplicaSet).map(Pick::from);
    let spec = set.spec.as_ref();
    template(ui, cx, spec.and_then(|spec| spec.template.as_ref()), true).or(pod)
}

pub(crate) fn controller_details(ui: &mut Ui, cx: &Context<'_>, set: &ReplicationController) -> Option<Link> {
    let (spec, status) = (set.spec.as_ref(), set.status.as_ref());
    details_heading(ui, cx);
    counts(
        ui,
        cx,
        &[
            ("desired", Some(spec.and_then(|spec| spec.replicas).unwrap_or(1))),
            ("current", status.map(|status| status.replicas)),
            ("ready", status.and_then(|status| status.ready_replicas)),
            ("available", status.and_then(|status| status.available_replicas)),
            ("labeled", status.and_then(|status| status.fully_labeled_replicas)),
        ],
    );
    let labels = spec
        .and_then(|spec| spec.selector.as_ref())
        .map(|selector| selector.iter().map(|(key, value)| format!("{key}={value}")).collect::<Vec<_>>().join(","));
    line(ui, cx, "Selector", labels.filter(|labels| !labels.is_empty()));
    line(ui, cx, "Min ready", seconds(spec.and_then(|spec| spec.min_ready_seconds), Some(0)));
    let observed = status.and_then(|status| status.observed_generation);
    line(ui, cx, "Observed", observed.map(|generation| format!("generation {generation}")));
    ui.add_space(10.0);
    None
}

pub(crate) fn controller_parts(ui: &mut Ui, cx: &Context<'_>, set: &ReplicationController) -> Option<Pick> {
    let pod = pods::show(ui, cx, Kind::ReplicationController).map(Pick::from);
    let spec = set.spec.as_ref();
    template(ui, cx, spec.and_then(|spec| spec.template.as_ref()), true).or(pod)
}
