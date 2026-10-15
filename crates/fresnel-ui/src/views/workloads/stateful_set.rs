//! The sections of a StatefulSet: the replicas, the update strategy with its partition, the
//! Service, the pod management policy and the selector; then its pods, its revisions and
//! its template.

use egui::Ui;
use fresnel_core::kinds::Kind;
use k8s_openapi::api::apps::v1::StatefulSet;

use super::{counts, pods, revisions, template};
use crate::views::detail::lines::{details_heading, line, number, seconds, selector};
use crate::views::detail::sections::{Context, Pick};
use crate::views::objects::Link;

pub(crate) fn details(ui: &mut Ui, cx: &Context<'_>, set: &StatefulSet) -> Option<Link> {
    let (spec, status) = (set.spec.as_ref(), set.status.as_ref());
    details_heading(ui, cx);
    counts(
        ui,
        cx,
        &[
            ("desired", Some(spec.and_then(|spec| spec.replicas).unwrap_or(1))),
            ("ready", status.and_then(|status| status.ready_replicas)),
            ("up to date", status.and_then(|status| status.updated_replicas)),
            ("available", status.and_then(|status| status.available_replicas)),
            ("current", status.and_then(|status| status.current_replicas)),
        ],
    );
    let strategy = spec.and_then(|spec| spec.update_strategy.as_ref());
    let kind = strategy.and_then(|strategy| strategy.type_.clone()).unwrap_or_else(|| "RollingUpdate".to_owned());
    let rolling = strategy.and_then(|strategy| strategy.rolling_update.as_ref());
    let partition = rolling.and_then(|rolling| rolling.partition).filter(|partition| *partition > 0);
    let unavailable = rolling.and_then(|rolling| rolling.max_unavailable.as_ref()).map(number);
    let mut text = kind;
    if let Some(partition) = partition {
        text.push_str(&format!(", partition {partition}"));
    }
    if let Some(unavailable) = unavailable {
        text.push_str(&format!(", max unavailable {unavailable}"));
    }
    line(ui, cx, "Update strategy", Some(text));
    line(ui, cx, "Service", spec.and_then(|spec| spec.service_name.clone()).filter(|name| !name.is_empty()));
    line(
        ui,
        cx,
        "Pod management",
        spec.and_then(|spec| spec.pod_management_policy.clone()).or(Some("OrderedReady".to_owned())),
    );
    line(ui, cx, "Selector", Some(selector(spec.map(|spec| &spec.selector))));
    line(ui, cx, "Min ready", seconds(spec.and_then(|spec| spec.min_ready_seconds), Some(0)));
    let revision = status.and_then(|status| status.update_revision.clone());
    line(ui, cx, "Update revision", revision);
    let claims = spec
        .and_then(|spec| spec.volume_claim_templates.as_ref())
        .map(|claims| claims.iter().filter_map(|claim| claim.metadata.name.clone()).collect::<Vec<_>>().join(", "));
    if let Some(claims) = claims.filter(|claims| !claims.is_empty()) {
        line(ui, cx, "Volume claims", Some(claims));
    }
    ui.add_space(10.0);
    None
}

pub(crate) fn parts(ui: &mut Ui, cx: &Context<'_>, set: &StatefulSet) -> Option<Pick> {
    let pod = pods::show(ui, cx, Kind::StatefulSet).map(Pick::from);
    let picked = revisions::show(ui, cx).or(pod);
    let spec = set.spec.as_ref();
    template(ui, cx, spec.map(|spec| &spec.template)).map(Pick::from).or(picked)
}
