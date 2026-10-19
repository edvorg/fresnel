//! The Event: its description and its health value.
//!
//! - `problem`: the state now of the problem of a warning Event, from the live objects.

mod problem;

use std::borrow::Cow;
use std::time::SystemTime;

use k8s_openapi::api::core::v1::Event;

pub use problem::{ProblemNow, ProblemState, Witness};

use super::health::ObjectHealth;
use super::{Action, Column, Describe, Kind, Sidebar, Tier, Value};

impl Describe for Event {
    const ID: Kind = Kind::Event;
    const TITLE: &'static str = "Events";
    const SIDEBAR: Sidebar = Sidebar::Cluster;
    const TIER: Tier = Tier::A;
    const COLUMNS: &'static [Column<Self>] = &[
        Column::text("Type", 70.0, |event| Value::text(event.type_.as_deref())),
        Column::text("Reason", 120.0, |event| Value::text(event.reason.as_deref())),
        Column::text("Object", 200.0, |event| {
            let object = &event.involved_object;
            let kind = object.kind.as_deref().unwrap_or_default().to_ascii_lowercase();
            Value::Text(Cow::Owned(format!("{kind}/{}", object.name.as_deref().unwrap_or_default())))
        }),
        Column::text("Source", 160.0, source).hidden(),
        Column::text("Message", 320.0, |event| Value::text(event.message.as_deref().map(str::trim))),
        Column::number("Count", 56.0, |event| {
            let count =
                event.series.as_ref().and_then(|series| series.count).or(event.count).filter(|count| *count > 0);
            Value::Number(count.unwrap_or(1).into())
        }),
        Column::number("First seen", 90.0, |event: &Self| Value::time(event.first_timestamp.as_ref())).hidden(),
        Column::number("Last seen", 90.0, last_seen),
    ];
    const ACTIONS: &'static [Action] = &[];

    /// An Event of the type `Warning` is a warning, so the "problems" filter of the Event
    /// table shows the warnings.
    fn health(&self) -> ObjectHealth {
        if self.type_.as_deref() == Some("Warning") { ObjectHealth::Warning } else { ObjectHealth::Healthy }
    }
}

/// The part of the cluster that reported the Event, with the rule of kubectl: the
/// component and its host, or the controller and its instance.
fn source(event: &Event) -> Value<'_> {
    fn first<'a>(a: Option<&'a str>, b: Option<&'a str>) -> &'a str {
        a.filter(|text| !text.is_empty()).or(b).unwrap_or_default()
    }
    let source = event.source.as_ref();
    let component = first(source.and_then(|source| source.component.as_deref()), event.reporting_component.as_deref());
    let host = first(source.and_then(|source| source.host.as_deref()), event.reporting_instance.as_deref());
    match (component, host) {
        ("", _) => Value::text(Some(host)),
        (_, "") => Value::text(Some(component)),
        _ => Value::Text(Cow::Owned(format!("{component}, {host}"))),
    }
}

/// The last time the Event happened: of its series, or the last time of the Event, or the
/// time of an Event of the newer API.
fn last_seen(event: &Event) -> Value<'static> {
    let series = event.series.as_ref().and_then(|series| series.last_observed_time.as_ref());
    let micro = series.or(event.event_time.as_ref()).map(|time| SystemTime::from(time.0));
    match event.last_timestamp.as_ref() {
        Some(time) if series.is_none() => Value::time(Some(time)),
        _ => micro.map_or(Value::None, Value::Time),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture;
    use crate::kinds::ObjectHealth::{Healthy, Warning};

    #[test]
    fn an_event_of_the_type_warning_is_a_warning() {
        let events = fixture::all::<Event>();
        let of_crash = |reason: &str| {
            let fits = |event: &&Event| {
                event.involved_object.name.as_deref() == Some("crash") && event.reason.as_deref() == Some(reason)
            };
            events.iter().find(fits).expect("an Event of the pod `crash`").health()
        };
        assert_eq!((of_crash("Scheduled"), of_crash("BackOff")), (Healthy, Warning));
        assert_eq!(Event::default().health(), Healthy, "no type is no warning");
    }
}
