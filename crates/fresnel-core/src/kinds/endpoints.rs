//! The Endpoints: its description. Its objects change all the time, so it has a store only
//! while a view needs it (plan section 9.5).

use k8s_openapi::api::core::v1::{EndpointSubset, Endpoints};

use super::describe::by_metadata;
use super::health::ObjectHealth;
use super::{Action, Column, Describe, Kind, Sidebar, Tier, Value};

impl Describe for Endpoints {
    const ID: Kind = Kind::Endpoints;
    const TITLE: &'static str = "Endpoints";
    const SIDEBAR: Sidebar = Sidebar::Network;
    const TIER: Tier = Tier::C;
    /// Each address that is ready, with each port: `10.42.0.9:80`. A subset with no port
    /// gives its addresses only, as kubectl does.
    const COLUMNS: &'static [Column<Self>] = &[Column::text("Endpoints", 260.0, |endpoints| {
        Value::texts(subsets(endpoints).flat_map(addresses_with_ports))
    })];
    const ACTIONS: &'static [Action] = &[];

    fn health(&self) -> ObjectHealth {
        by_metadata(&self.metadata)
    }

    fn search<'a>(&'a self, texts: &mut Vec<&'a str>) {
        let addresses = subsets(self).flat_map(|subset| subset.addresses.iter().flatten());
        texts.extend(addresses.map(|address| address.ip.as_str()));
    }
}

fn subsets(endpoints: &Endpoints) -> impl Iterator<Item = &EndpointSubset> {
    endpoints.subsets.iter().flatten()
}

fn addresses_with_ports(subset: &EndpointSubset) -> Vec<String> {
    let addresses = || subset.addresses.iter().flatten().map(|address| address.ip.as_str());
    let ports: Vec<i32> = subset.ports.iter().flatten().map(|port| port.port).collect();
    if ports.is_empty() {
        return addresses().map(str::to_owned).collect();
    }
    let host = |ip: &str| if ip.contains(':') { format!("[{ip}]") } else { ip.to_owned() };
    ports.iter().flat_map(|port| addresses().map(move |ip| format!("{}:{port}", host(ip)))).collect()
}
