//! The sections of a NetworkPolicy (`research/01` section 3.4.3): the pods that it selects,
//! its policy types, and each rule of its incoming and outgoing traffic in words, as
//! `kubectl describe` tells it: "From pods app=web in namespaces team=a, to port TCP 80".
//!
//! A policy type with no rule denies each connection of that way. A way that the policy
//! types do not name is not a part of the policy: the text tells the two cases apart.

use egui::{Label, Ui};
use k8s_openapi::api::networking::v1::{NetworkPolicy, NetworkPolicyPeer, NetworkPolicyPort};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector;

use crate::format::short;
use crate::views::detail::fields::{ITEM_LIMIT, VALUE_LIMIT, field, heading, more, note};
use crate::views::detail::lines::{details_heading, line, narrows, number, selector};
use crate::views::detail::sections::Context;
use crate::views::objects::Link;

/// One rule of one way: its peers and its ports.
struct Rule<'a> {
    peers: &'a [NetworkPolicyPeer],
    ports: &'a [NetworkPolicyPort],
}

pub(crate) fn details(ui: &mut Ui, cx: &Context<'_>, policy: &NetworkPolicy) -> Option<Link> {
    let spec = policy.spec.as_ref();
    details_heading(ui, cx);
    let pods = spec.and_then(|spec| spec.pod_selector.as_ref());
    line(ui, cx, "Pods", Some(pods_of(pods, "each pod of the namespace")));
    let ingress: Vec<Rule<'_>> = spec
        .and_then(|spec| spec.ingress.as_deref())
        .unwrap_or_default()
        .iter()
        .map(|rule| Rule {
            peers: rule.from.as_deref().unwrap_or_default(),
            ports: rule.ports.as_deref().unwrap_or_default(),
        })
        .collect();
    let egress: Vec<Rule<'_>> = spec
        .and_then(|spec| spec.egress.as_deref())
        .unwrap_or_default()
        .iter()
        .map(|rule| Rule {
            peers: rule.to.as_deref().unwrap_or_default(),
            ports: rule.ports.as_deref().unwrap_or_default(),
        })
        .collect();
    // With no policy types, a policy is for the incoming traffic, and for the outgoing
    // traffic when it has a rule of it.
    let types = spec.and_then(|spec| spec.policy_types.clone());
    let names = |name: &str| {
        types.as_ref().map_or(name == "Ingress" || !egress.is_empty(), |types| types.iter().any(|kind| kind == name))
    };
    let (incoming, outgoing) = (names("Ingress"), names("Egress"));
    let types: Vec<&str> =
        [("Ingress", incoming), ("Egress", outgoing)].into_iter().filter(|(_, on)| *on).map(|(name, _)| name).collect();
    line(ui, cx, "Policy types", Some(types.join(", ")));
    ui.add_space(10.0);
    way(ui, cx, Way { title: "INCOMING TRAFFIC", from: "From", on: incoming, rules: &ingress });
    way(ui, cx, Way { title: "OUTGOING TRAFFIC", from: "To", on: outgoing, rules: &egress });
    None
}

/// The rules of one way of the traffic.
struct Way<'a> {
    title: &'static str,
    /// "From" or "To".
    from: &'static str,
    /// True when the policy types name the way.
    on: bool,
    rules: &'a [Rule<'a>],
}

fn way(ui: &mut Ui, cx: &Context<'_>, way: Way<'_>) {
    heading(ui, cx.palette, way.title);
    if !way.on {
        note(ui, cx.palette.text_muted, "Not a part of the policy: the policy does not change this traffic.");
    } else if way.rules.is_empty() {
        note(ui, cx.palette.warn, "No rule: the selected pods take no connection of this way.");
    }
    for (number, rule) in way.rules.iter().enumerate().take(ITEM_LIMIT) {
        field(ui, cx.palette, &format!("Rule {}", number + 1), |ui| {
            let what = if way.from == "From" { "each source" } else { "each destination" };
            let peers: Vec<String> = rule.peers.iter().map(peer).collect();
            let peers = if peers.is_empty() { what.to_owned() } else { peers.join("; ") };
            ui.add(Label::new(short(&format!("{} {peers}", way.from), VALUE_LIMIT)).wrap());
            ui.add(Label::new(short(&format!("Ports: {}", ports(rule.ports)), VALUE_LIMIT)).wrap());
        });
    }
    more(ui, cx.palette, way.rules.len());
    ui.add_space(10.0);
}

/// The pods of a selector, or `each` for a selector with no requirement.
fn pods_of(pods: Option<&LabelSelector>, each: &str) -> String {
    match pods.filter(|pods| narrows(pods)) {
        Some(pods) => format!("pods {}", selector(Some(pods))),
        None => each.to_owned(),
    }
}

/// One peer of a rule in words.
fn peer(peer: &NetworkPolicyPeer) -> String {
    if let Some(block) = &peer.ip_block {
        let except = block.except.as_deref().unwrap_or_default();
        return match except.is_empty() {
            true => format!("IP block {}", block.cidr),
            false => format!("IP block {} except {}", block.cidr, except.join(", ")),
        };
    }
    let namespaces = peer.namespace_selector.as_ref().map(|namespaces| match narrows(namespaces) {
        true => format!("in namespaces {}", selector(Some(namespaces))),
        false => "in each namespace".to_owned(),
    });
    let pods = pods_of(peer.pod_selector.as_ref(), "each pod");
    match namespaces {
        Some(namespaces) => format!("{pods} {namespaces}"),
        None => format!("{pods} of the namespace of the policy"),
    }
}

/// The ports of a rule: "TCP 80, UDP 53, TCP 8000-9000", or each port.
fn ports(ports: &[NetworkPolicyPort]) -> String {
    if ports.is_empty() {
        return "each port".to_owned();
    }
    let text = |port: &NetworkPolicyPort| {
        let protocol = port.protocol.as_deref().unwrap_or("TCP");
        match (port.port.as_ref().map(number), port.end_port) {
            (None, _) => format!("{protocol} each port"),
            (Some(first), Some(last)) => format!("{protocol} {first}-{last}"),
            (Some(port), None) => format!("{protocol} {port}"),
        }
    };
    ports.iter().map(text).collect::<Vec<_>>().join(", ")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn each_peer_and_each_port_reads_in_words() {
        let peers: Vec<NetworkPolicyPeer> = serde_json::from_value(json!([
            {"podSelector": {"matchLabels": {"app": "web"}}},
            {"namespaceSelector": {"matchLabels": {"team": "a"}}},
            {"podSelector": {"matchLabels": {"app": "db"}}, "namespaceSelector": {}},
            {"podSelector": {}},
            {"ipBlock": {"cidr": "10.0.0.0/8", "except": ["10.1.0.0/16"]}},
        ]))
        .expect("peers");
        let texts: Vec<String> = peers.iter().map(peer).collect();
        assert_eq!(
            texts,
            [
                "pods app=web of the namespace of the policy",
                "each pod in namespaces team=a",
                "pods app=db in each namespace",
                "each pod of the namespace of the policy",
                "IP block 10.0.0.0/8 except 10.1.0.0/16",
            ]
        );
        let list: Vec<NetworkPolicyPort> = serde_json::from_value(json!([
            {"port": 80}, {"protocol": "UDP", "port": "dns"}, {"port": 8000, "endPort": 9000}, {"protocol": "SCTP"}
        ]))
        .expect("ports");
        assert_eq!(ports(&list), "TCP 80, UDP dns, TCP 8000-9000, SCTP each port");
        assert_eq!(ports(&[]), "each port");
    }
}
