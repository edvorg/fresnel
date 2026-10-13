//! The Event: its health value.

use k8s_openapi::api::core::v1::Event;

use super::health::{Assess, ObjectHealth};

impl Assess for Event {
    /// An Event of the type `Warning` is a warning, so the "problems" filter of the Event
    /// table shows the warnings.
    fn health(&self) -> ObjectHealth {
        if self.type_.as_deref() == Some("Warning") { ObjectHealth::Warning } else { ObjectHealth::Healthy }
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
