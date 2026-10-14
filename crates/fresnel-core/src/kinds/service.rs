//! The Service: its description and its health value.

use std::borrow::Cow;

use k8s_openapi::api::core::v1::{LoadBalancerIngress, Service, ServiceSpec};

use super::health::ObjectHealth;
use super::{Action, Column, Describe, Kind, Sidebar, Tier, Value, read};

impl Describe for Service {
    const ID: Kind = Kind::Service;
    const TITLE: &'static str = "Services";
    const SIDEBAR: Sidebar = Sidebar::Network;
    const TIER: Tier = Tier::A;
    const COLUMNS: &'static [Column<Self>] = &[
        Column::text("Type", 100.0, |service| Value::text(spec(service).and_then(|spec| spec.type_.as_deref()))),
        Column::text("Cluster IP", 110.0, |service| {
            Value::text(spec(service).and_then(|spec| spec.cluster_ip.as_deref()))
        }),
        Column::text("External IP", 130.0, external),
        Column::text("Ports", 140.0, |service| {
            let ports = spec(service).and_then(|spec| spec.ports.as_ref()).into_iter().flatten();
            Value::texts(ports.map(port_text))
        }),
        Column::text("Selector", 160.0, |service| read::labels(spec(service).and_then(|spec| spec.selector.as_ref()))),
    ];
    const ACTIONS: &'static [Action] = &[Action::Forward];

    /// The addresses of the Service, and the name of an `ExternalName` Service.
    fn search<'a>(&'a self, texts: &mut Vec<&'a str>) {
        let spec = spec(self);
        texts.extend(spec.into_iter().flat_map(|spec| spec.cluster_ips.iter().flatten()).map(String::as_str));
        texts.extend(spec.into_iter().flat_map(|spec| spec.external_ips.iter().flatten()).map(String::as_str));
        texts.extend(spec.and_then(|spec| spec.external_name.as_deref()));
        texts.extend(balancers(self).filter_map(address));
    }

    /// A Service of the type `LoadBalancer` is on its way until it has an address. Each
    /// other Service is healthy: the pods behind a Service show in its EndpointSlices.
    fn health(&self) -> ObjectHealth {
        let balancer = self.spec.as_ref().and_then(|spec| spec.type_.as_deref()) == Some("LoadBalancer");
        if balancer && balancers(self).next().is_none() { ObjectHealth::Progressing } else { ObjectHealth::Healthy }
    }
}

fn spec(service: &Service) -> Option<&ServiceSpec> {
    service.spec.as_ref()
}

/// The entries of the load balancer of the Service.
fn balancers(service: &Service) -> impl Iterator<Item = &LoadBalancerIngress> {
    let status = service.status.as_ref().and_then(|status| status.load_balancer.as_ref());
    status.into_iter().flat_map(|balancer| balancer.ingress.iter().flatten())
}

/// The address of one entry of a load balancer: its IP address, or else its host name.
fn address(entry: &LoadBalancerIngress) -> Option<&str> {
    entry.ip.as_deref().filter(|ip| !ip.is_empty()).or(entry.hostname.as_deref())
}

/// The addresses outside the cluster, with the rule of kubectl: the addresses of the load
/// balancer and the external IPs. A load balancer with no address yet is `pending`, and
/// an `ExternalName` Service shows its name.
fn external(service: &Service) -> Value<'_> {
    let Some(spec) = spec(service) else {
        return Value::None;
    };
    let external_ips = spec.external_ips.iter().flatten().map(String::as_str);
    match spec.type_.as_deref() {
        Some("LoadBalancer") => {
            let all: Vec<&str> = balancers(service).filter_map(address).chain(external_ips).collect();
            if all.is_empty() { Value::Text(Cow::Borrowed("pending")) } else { Value::list(all) }
        }
        Some("ExternalName") => Value::text(spec.external_name.as_deref()),
        _ => Value::list(external_ips),
    }
}

/// One port as kubectl prints it: `80/TCP`, or `80:30080/TCP` with a node port.
fn port_text(port: &k8s_openapi::api::core::v1::ServicePort) -> String {
    let protocol = port.protocol.as_deref().unwrap_or("TCP");
    match port.node_port.filter(|node_port| *node_port > 0) {
        Some(node_port) => format!("{}:{node_port}/{protocol}", port.port),
        None => format!("{}/{protocol}", port.port),
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
