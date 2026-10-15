//! The sections of an Event (`research/02` section 4.5): its type, its reason, its message,
//! its object and its related object as links, the part of the cluster that reported it,
//! its count and the times of its first and its last report.

use std::time::SystemTime;

use egui::{Label, RichText, Ui};
use fresnel_core::index::last_seen;
use k8s_openapi::api::core::v1::{Event, ObjectReference};

use crate::views::detail::fields::{NONE, field};
use crate::views::detail::lines::{details_heading, line};
use crate::views::detail::sections::Context;
use crate::views::objects::Link;
use crate::views::pod_spec::texts::{at, moment};

/// The most characters of the message. A recorder of Kubernetes cuts a message at 1,024
/// bytes; some other writer of an Event can send more.
const MESSAGE_LIMIT: usize = 2000;

pub(crate) fn details(ui: &mut Ui, cx: &Context<'_>, event: &Event) -> Option<Link> {
    details_heading(ui, cx);
    let kind = event.type_.as_deref().unwrap_or(NONE);
    let color = if kind == "Warning" { cx.palette.warn } else { cx.palette.text };
    field(ui, cx.palette, "Type", |ui| {
        ui.label(RichText::new(kind).color(color));
    });
    line(ui, cx, "Reason", event.reason.clone());
    let message = event.message.as_deref().map(str::trim).filter(|message| !message.is_empty());
    field(ui, cx.palette, "Message", |ui| {
        ui.add(Label::new(crate::format::short(message.unwrap_or(NONE), MESSAGE_LIMIT)).wrap());
    });
    let mut clicked = None;
    field(ui, cx.palette, "Object", |ui| clicked = object(ui, cx, &event.involved_object));
    if let Some(related) = &event.related {
        field(ui, cx.palette, "Related", |ui| clicked = object(ui, cx, related).or(clicked.take()));
    }
    let source = event.source.as_ref();
    let component = source.and_then(|source| source.component.clone()).or(event.reporting_component.clone());
    let host = source.and_then(|source| source.host.clone()).or(event.reporting_instance.clone());
    let reporter = [component, host].into_iter().flatten().filter(|text| !text.is_empty()).collect::<Vec<_>>();
    line(ui, cx, "Source", (!reporter.is_empty()).then(|| reporter.join(", ")));
    line(ui, cx, "Action", event.action.clone());
    let count = event.series.as_ref().and_then(|series| series.count).or(event.count).unwrap_or(1);
    line(ui, cx, "Count", Some(count.to_string()));
    let first = event.first_timestamp.as_ref().map(|time| at(time, cx.clock));
    let first = first.or_else(|| event.event_time.as_ref().map(|time| moment(SystemTime::from(time.0), cx.clock)));
    line(ui, cx, "First seen", first);
    line(ui, cx, "Last seen", last_seen(event).map(|time| moment(SystemTime::from(time), cx.clock)));
    ui.add_space(10.0);
    clicked
}

/// The object of an Event as a link. An Event in a namespace is about an object of that
/// namespace, unless its reference names an other one.
fn object(ui: &mut Ui, cx: &Context<'_>, reference: &ObjectReference) -> Option<Link> {
    let (Some(kind), Some(name)) = (reference.kind.as_deref(), reference.name.as_deref()) else {
        ui.label(NONE);
        return None;
    };
    let namespace = reference.namespace.as_deref().unwrap_or(&cx.id.key.namespace);
    cx.reference(ui, kind, namespace, name)
}
