//! The Service: its health value.

use k8s_openapi::api::core::v1::Service;

use super::health::{Assess, ObjectHealth};

impl Assess for Service {
    /// A Service of the type `LoadBalancer` is on its way until it has an address. Each
    /// other Service is healthy: the pods behind a Service show in its EndpointSlices.
    fn health(&self) -> ObjectHealth {
        let balancer = self.spec.as_ref().and_then(|spec| spec.type_.as_deref()) == Some("LoadBalancer");
        let addresses = self.status.as_ref().and_then(|status| status.load_balancer.as_ref()?.ingress.as_ref());
        if balancer && addresses.is_none_or(Vec::is_empty) { ObjectHealth::Progressing } else { ObjectHealth::Healthy }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::fixture;
    use crate::kinds::ObjectHealth::{Healthy, Progressing};

    #[test]
    fn a_load_balancer_is_on_its_way_until_it_has_an_address() {
        let health = |name: &str| fixture::one::<Service>(name).health();
        assert_eq!(
            (health("web"), health("db"), health("outside"), health("nobody")),
            (Healthy, Healthy, Healthy, Healthy)
        );
        assert_eq!(health("public"), Progressing, "the test cluster has no load balancer");
        let with_address = fixture::changed::<Service>("public", |service| {
            service["status"] = json!({ "loadBalancer": { "ingress": [{ "ip": "192.0.2.7" }] } });
        });
        assert_eq!(with_address.health(), Healthy);
    }
}
