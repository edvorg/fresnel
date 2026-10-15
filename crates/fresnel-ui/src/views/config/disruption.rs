//! The sections of a PodDisruptionBudget (`research/01` section 3.4.5, `research/02` section
//! 4.3): the budget, the selector and the policy for unhealthy pods; then the counts of its
//! status, which tell if an eviction can go now, and the pods that an eviction takes now.
//!
//! A drain evicts each pod, and a budget with no allowed disruption blocks it: the view
//! names the cause in one line.

use egui::{RichText, Ui};
use fresnel_core::kinds::Kind;
use k8s_openapi::api::policy::v1::PodDisruptionBudget;

use crate::views::detail::fields::{ITEM_LIMIT, field, more};
use crate::views::detail::lines::{details_heading, line, number, selector};
use crate::views::detail::sections::Context;
use crate::views::objects::Link;
use crate::views::pod_spec::texts::at;

pub(crate) fn details(ui: &mut Ui, cx: &Context<'_>, budget: &PodDisruptionBudget) -> Option<Link> {
    let (spec, status) = (budget.spec.as_ref(), budget.status.as_ref());
    details_heading(ui, cx);
    let min = spec.and_then(|spec| spec.min_available.as_ref()).map(number);
    let max = spec.and_then(|spec| spec.max_unavailable.as_ref()).map(number);
    match (min, max) {
        (Some(min), None) => line(ui, cx, "Min available", Some(min)),
        (None, Some(max)) => line(ui, cx, "Max unavailable", Some(max)),
        (min, max) => {
            line(ui, cx, "Min available", min);
            line(ui, cx, "Max unavailable", max);
        }
    }
    line(ui, cx, "Selector", Some(selector(spec.and_then(|spec| spec.selector.as_ref()))));
    let policy = spec.and_then(|spec| spec.unhealthy_pod_eviction_policy.clone());
    line(ui, cx, "Unhealthy pods", Some(policy.unwrap_or_else(|| "IfHealthyBudget".to_owned())));
    let Some(status) = status else {
        line(ui, cx, "Disruptions", Some("not known yet: the controller wrote no status".to_owned()));
        ui.add_space(10.0);
        return None;
    };
    let (allowed, tone) = match status.disruptions_allowed {
        0 => ("none allowed: an eviction of a pod waits".to_owned(), cx.palette.warn),
        allowed => (format!("{allowed} allowed"), cx.palette.text),
    };
    field(ui, cx.palette, "Disruptions", |ui| {
        ui.label(RichText::new(allowed).color(tone));
    });
    let healthy = format!(
        "{} current, {} desired, of {} expected pods",
        status.current_healthy, status.desired_healthy, status.expected_pods
    );
    line(ui, cx, "Healthy", Some(healthy));
    let disrupted = status.disrupted_pods.iter().flatten().collect::<Vec<_>>();
    let mut clicked = None;
    if !disrupted.is_empty() {
        field(ui, cx.palette, "Evicted now", |ui| {
            for (name, time) in disrupted.iter().take(ITEM_LIMIT) {
                ui.horizontal(|ui| {
                    clicked = cx.link(ui, name, Kind::Pod, &cx.id.key.namespace, name).or(clicked.take());
                    ui.label(RichText::new(format!("since {}", at(time, cx.clock))).color(cx.palette.text_muted));
                });
            }
            more(ui, cx.palette, disrupted.len());
        });
    }
    ui.add_space(10.0);
    clicked
}
