//! The sections of a Lease (`research/01` section 3.4.5, `research/02` section 4.3): its
//! holder, its duration, the times of its acquire and of its last renewal, and its
//! transitions. The view adds the end of the lease: a holder that did not renew in time
//! lost it, and a leader election then picks a new one.

use std::time::{Duration, SystemTime};

use egui::{RichText, Ui};
use k8s_openapi::api::coordination::v1::Lease;

use crate::format;
use crate::views::detail::fields::field;
use crate::views::detail::lines::{details_heading, line};
use crate::views::detail::sections::Context;
use crate::views::objects::Link;
use crate::views::pod_spec::texts::moment;

pub(crate) fn details(ui: &mut Ui, cx: &Context<'_>, lease: &Lease) -> Option<Link> {
    let spec = lease.spec.as_ref();
    details_heading(ui, cx);
    let holder = spec.and_then(|spec| spec.holder_identity.clone()).filter(|holder| !holder.is_empty());
    line(ui, cx, "Holder", Some(holder.unwrap_or_else(|| "none".to_owned())));
    let duration = spec.and_then(|spec| spec.lease_duration_seconds);
    line(ui, cx, "Duration", duration.map(|seconds| format!("{seconds} s")));
    let time = |time: Option<&k8s_openapi::apimachinery::pkg::apis::meta::v1::MicroTime>| {
        time.map(|time| SystemTime::from(time.0))
    };
    let (acquired, renewed) =
        (time(spec.and_then(|spec| spec.acquire_time.as_ref())), time(spec.and_then(|spec| spec.renew_time.as_ref())));
    line(ui, cx, "Acquired", acquired.map(|time| moment(time, cx.clock)));
    line(ui, cx, "Renewed", renewed.map(|time| moment(time, cx.clock)));
    if let (Some(renewed), Some(seconds)) = (renewed, duration) {
        let end = renewed + Duration::from_secs(u64::try_from(seconds).unwrap_or(0));
        let (text, color) = match cx.clock.until(end) {
            Duration::ZERO => (
                format!("ended {} ago: the holder did not renew it", format::span(cx.clock.since(end))),
                cx.palette.warn,
            ),
            left => (format!("in {}", format::span(left)), cx.palette.text),
        };
        field(ui, cx.palette, "Ends", |ui| {
            ui.label(RichText::new(text).color(color));
        });
    }
    line(ui, cx, "Transitions", spec.and_then(|spec| spec.lease_transitions).map(|count| count.to_string()));
    let optional = [
        ("Strategy", spec.and_then(|spec| spec.strategy.clone())),
        ("Preferred holder", spec.and_then(|spec| spec.preferred_holder.clone())),
    ];
    for (label, value) in optional {
        if value.is_some() {
            line(ui, cx, label, value);
        }
    }
    ui.add_space(10.0);
    None
}
