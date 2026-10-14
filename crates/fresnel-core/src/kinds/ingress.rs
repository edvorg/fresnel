//! The Ingress: its description.

use std::borrow::Cow;

use k8s_openapi::api::networking::v1::{Ingress, IngressRule};

use super::describe::by_metadata;
use super::health::ObjectHealth;
use super::{Action, Column, Describe, Kind, Sidebar, Tier, Value};

impl Describe for Ingress {
    const ID: Kind = Kind::Ingress;
    const TITLE: &'static str = "Ingresses";
    const SIDEBAR: Sidebar = Sidebar::Network;
    const TIER: Tier = Tier::B;
    const COLUMNS: &'static [Column<Self>] = &[
        Column::text("Class", 100.0, |ingress| {
            Value::text(ingress.spec.as_ref().and_then(|spec| spec.ingress_class_name.as_deref()))
        }),
        Column::text("Hosts", 200.0, |ingress| {
            // A rule with no host takes each host, which kubectl shows as `*`.
            let rules: Vec<&IngressRule> = rules(ingress).collect();
            let hosts =
                Value::list(rules.iter().filter_map(|rule| rule.host.as_deref()).filter(|host| !host.is_empty()));
            if hosts == Value::None { Value::Text(Cow::Borrowed("*")) } else { hosts }
        }),
        Column::text("Paths", 160.0, |ingress| {
            let paths = rules(ingress).filter_map(|rule| rule.http.as_ref()).flat_map(|http| &http.paths);
            Value::list(paths.filter_map(|path| path.path.as_deref()))
        }),
        Column::text("Address", 140.0, |ingress| Value::list(addresses(ingress))),
        Column::text("Ports", 70.0, |ingress| {
            let tls = ingress.spec.as_ref().and_then(|spec| spec.tls.as_ref()).is_some_and(|tls| !tls.is_empty());
            Value::Text(Cow::Borrowed(if tls { "80, 443" } else { "80" }))
        }),
    ];
    const ACTIONS: &'static [Action] = &[];

    fn health(&self) -> ObjectHealth {
        by_metadata(&self.metadata)
    }

    /// The hosts of the rules and the addresses of the load balancer.
    fn search<'a>(&'a self, texts: &mut Vec<&'a str>) {
        texts.extend(rules(self).filter_map(|rule| rule.host.as_deref()));
        texts.extend(addresses(self));
    }
}

fn rules(ingress: &Ingress) -> impl Iterator<Item = &IngressRule> {
    ingress.spec.as_ref().and_then(|spec| spec.rules.as_ref()).into_iter().flatten()
}

/// The address of each entry of the load balancer: its IP address, or else its host name.
fn addresses(ingress: &Ingress) -> impl Iterator<Item = &str> {
    let balancer = ingress.status.as_ref().and_then(|status| status.load_balancer.as_ref());
    let entries = balancer.and_then(|balancer| balancer.ingress.as_ref()).into_iter().flatten();
    entries.filter_map(|entry| entry.ip.as_deref().filter(|ip| !ip.is_empty()).or(entry.hostname.as_deref()))
}
