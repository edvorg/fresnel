//! The sections of a Deployment: the replicas, the strategy with its numbers, the selector,
//! the deadlines and the history limit; then its pods, its revisions and its template.

use egui::Ui;
use fresnel_core::kinds::Kind;
use k8s_openapi::api::apps::v1::Deployment;

use super::{counts, pods, revisions, template};
use crate::views::detail::lines::{details_heading, line, number, seconds, selector};
use crate::views::detail::sections::{Context, Pick};
use crate::views::objects::Link;

pub(crate) fn details(ui: &mut Ui, cx: &Context<'_>, deployment: &Deployment) -> Option<Link> {
    let (spec, status) = (deployment.spec.as_ref(), deployment.status.as_ref());
    details_heading(ui, cx);
    counts(
        ui,
        cx,
        &[
            ("desired", Some(spec.and_then(|spec| spec.replicas).unwrap_or(1))),
            ("ready", status.and_then(|status| status.ready_replicas)),
            ("up to date", status.and_then(|status| status.updated_replicas)),
            ("available", status.and_then(|status| status.available_replicas)),
            ("total", status.and_then(|status| status.replicas)),
        ],
    );
    line(ui, cx, "Strategy", spec.map(|spec| strategy(spec.strategy.as_ref())));
    line(ui, cx, "Selector", Some(selector(spec.map(|spec| &spec.selector))));
    line(ui, cx, "Min ready", seconds(spec.and_then(|spec| spec.min_ready_seconds), Some(0)));
    line(ui, cx, "Progress deadline", seconds(spec.and_then(|spec| spec.progress_deadline_seconds), Some(600)));
    let limit = spec.and_then(|spec| spec.revision_history_limit).unwrap_or(10);
    line(ui, cx, "History limit", Some(format!("{limit} revisions")));
    if spec.and_then(|spec| spec.paused) == Some(true) {
        line(ui, cx, "Paused", Some("yes: a change of the template starts no rollout".to_owned()));
    }
    ui.add_space(10.0);
    None
}

pub(crate) fn parts(ui: &mut Ui, cx: &Context<'_>, deployment: &Deployment) -> Option<Pick> {
    let pod = pods::show(ui, cx, Kind::Deployment).map(Pick::from);
    let picked = revisions::show(ui, cx).or(pod);
    let spec = deployment.spec.as_ref();
    template(ui, cx, spec.map(|spec| &spec.template)).map(Pick::from).or(picked)
}

/// "RollingUpdate (max surge 25%, max unavailable 25%)" or "Recreate".
fn strategy(strategy: Option<&k8s_openapi::api::apps::v1::DeploymentStrategy>) -> String {
    let kind = strategy.and_then(|strategy| strategy.type_.as_deref()).unwrap_or("RollingUpdate");
    let rolling = strategy.and_then(|strategy| strategy.rolling_update.as_ref());
    if kind != "RollingUpdate" {
        return kind.to_owned();
    }
    let surge = rolling.and_then(|rolling| rolling.max_surge.as_ref()).map_or_else(|| "25%".to_owned(), number);
    let unavailable =
        rolling.and_then(|rolling| rolling.max_unavailable.as_ref()).map_or_else(|| "25%".to_owned(), number);
    format!("{kind} (max surge {surge}, max unavailable {unavailable})")
}
