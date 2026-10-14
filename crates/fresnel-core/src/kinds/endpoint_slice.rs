//! The EndpointSlice: its description, its health value, and the state of one endpoint.

use k8s_openapi::api::discovery::v1::{Endpoint, EndpointSlice};

use super::health::ObjectHealth;
use super::{Action, Column, Describe, Kind, Sidebar, Tier, Value};

impl Describe for EndpointSlice {
    const ID: Kind = Kind::EndpointSlice;
    const TITLE: &'static str = "EndpointSlices";
    const SIDEBAR: Sidebar = Sidebar::Network;
    const TIER: Tier = Tier::A;
    const COLUMNS: &'static [Column<Self>] = &[
        Column::text("Address type", 90.0, |slice| Value::text(Some(&slice.address_type))),
        Column::text("Ports", 120.0, |slice| {
            let port = |port: &k8s_openapi::api::discovery::v1::EndpointPort| match (port.port, &port.name) {
                (Some(number), _) => number.to_string(),
                (None, Some(name)) => name.clone(),
                (None, None) => "*".to_owned(),
            };
            Value::texts(slice.ports.iter().flatten().map(port))
        }),
        Column::text("Endpoints", 220.0, |slice| Value::list(addresses(slice))),
    ];
    const ACTIONS: &'static [Action] = &[];

    /// An endpoint that ends does not count: it leaves the slice. Of the other ones, one
    /// that is not ready is a warning, and the slice failed when none is ready. A slice
    /// with no endpoint is healthy: a Service can have no pod by intent.
    fn health(&self) -> ObjectHealth {
        let stay = || self.endpoints.iter().filter(|endpoint| !ends(endpoint));
        let (all, ready) = (stay().count(), stay().filter(|endpoint| is_ready(endpoint)).count());
        if ready == all {
            ObjectHealth::Healthy
        } else if ready == 0 {
            ObjectHealth::Failed
        } else {
            ObjectHealth::Warning
        }
    }

    fn search<'a>(&'a self, texts: &mut Vec<&'a str>) {
        texts.extend(addresses(self));
    }
}

/// The addresses of each endpoint of the slice.
fn addresses(slice: &EndpointSlice) -> impl Iterator<Item = &str> {
    slice.endpoints.iter().flat_map(|endpoint| &endpoint.addresses).map(String::as_str)
}

/// True for an endpoint that takes new connections. The API tells to read an absent
/// `ready` as ready. An endpoint that ends is not ready.
pub(crate) fn is_ready(endpoint: &Endpoint) -> bool {
    let conditions = endpoint.conditions.as_ref();
    conditions.and_then(|conditions| conditions.ready) != Some(false) && !ends(endpoint)
}

fn ends(endpoint: &Endpoint) -> bool {
    endpoint.conditions.as_ref().and_then(|conditions| conditions.terminating) == Some(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture;
    use crate::kinds::ObjectHealth::{Failed, Healthy, Warning};

    #[test]
    fn a_slice_warns_for_an_endpoint_that_is_not_ready_and_does_not_count_one_that_ends() {
        let health = |name: &str| fixture::one::<EndpointSlice>(name).health();
        assert_eq!((health("web-"), health("db-")), (Healthy, Healthy));
        assert_eq!(health("nobody-"), Healthy, "no pod has the labels of this Service");
        // The pods `sidecar` (ready), `not-ready` and `leaving` (it ends) are behind `mixed`.
        let mixed = fixture::one::<EndpointSlice>("mixed-");
        let states: Vec<(bool, bool)> =
            mixed.endpoints.iter().map(|endpoint| (is_ready(endpoint), ends(endpoint))).collect();
        assert_eq!(states.iter().filter(|state| **state == (true, false)).count(), 1, "{states:?}");
        assert_eq!(states.iter().filter(|state| **state == (false, false)).count(), 1, "{states:?}");
        assert_eq!(states.iter().filter(|state| **state == (false, true)).count(), 1, "{states:?}");
        assert_eq!(mixed.health(), Warning);

        let none_ready = fixture::changed::<EndpointSlice>("web-", |slice| {
            for endpoint in slice["endpoints"].as_array_mut().expect("endpoints") {
                endpoint["conditions"]["ready"] = false.into();
            }
        });
        assert_eq!(none_ready.health(), Failed);
        let no_conditions = fixture::changed::<EndpointSlice>("web-", |slice| {
            slice["endpoints"][0].as_object_mut().expect("an endpoint").remove("conditions");
        });
        assert_eq!(no_conditions.health(), Healthy, "an absent `ready` means ready");
    }
}
