//! The sections of a DaemonSet: the counts of its nodes, the update strategy and the
//! selector; then its pods, its revisions and its template with its node selector.

use egui::Ui;
use fresnel_core::kinds::Kind;
use k8s_openapi::api::apps::v1::DaemonSet;

use super::{pods, revisions, template};
use crate::views::detail::fields::text_field;
use crate::views::detail::lines::{details_heading, line, number, seconds, selector};
use crate::views::detail::sections::{Context, Pick};
use crate::views::objects::Link;

pub(crate) fn details(ui: &mut Ui, cx: &Context<'_>, set: &DaemonSet) -> Option<Link> {
    let (spec, status) = (set.spec.as_ref(), set.status.as_ref());
    details_heading(ui, cx);
    let count = |value: Option<i32>| value.unwrap_or(0);
    let nodes = status.map_or_else(String::new, |status| {
        format!(
            "{} desired, {} current, {} ready, {} up to date, {} available, {} misscheduled",
            status.desired_number_scheduled,
            status.current_number_scheduled,
            status.number_ready,
            count(status.updated_number_scheduled),
            count(status.number_available),
            status.number_misscheduled,
        )
    });
    text_field(ui, cx.palette, "Nodes", if nodes.is_empty() { "no status yet" } else { &nodes });
    let strategy = spec.and_then(|spec| spec.update_strategy.as_ref());
    let kind = strategy.and_then(|strategy| strategy.type_.clone()).unwrap_or_else(|| "RollingUpdate".to_owned());
    let rolling = strategy.and_then(|strategy| strategy.rolling_update.as_ref());
    let mut text = kind;
    if let Some(unavailable) = rolling.and_then(|rolling| rolling.max_unavailable.as_ref()) {
        text.push_str(&format!(", max unavailable {}", number(unavailable)));
    }
    if let Some(surge) = rolling.and_then(|rolling| rolling.max_surge.as_ref()) {
        text.push_str(&format!(", max surge {}", number(surge)));
    }
    line(ui, cx, "Update strategy", Some(text));
    line(ui, cx, "Selector", Some(selector(spec.map(|spec| &spec.selector))));
    line(ui, cx, "Min ready", seconds(spec.and_then(|spec| spec.min_ready_seconds), Some(0)));
    ui.add_space(10.0);
    None
}

pub(crate) fn parts(ui: &mut Ui, cx: &Context<'_>, set: &DaemonSet) -> Option<Pick> {
    let pod = pods::show(ui, cx, Kind::DaemonSet).map(Pick::from);
    let picked = revisions::show(ui, cx).or(pod);
    let spec = set.spec.as_ref();
    template(ui, cx, spec.map(|spec| &spec.template)).map(Pick::from).or(picked)
}
