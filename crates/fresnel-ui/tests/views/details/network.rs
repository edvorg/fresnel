//! The views of the Network kinds: a Service with its endpoints from the owner index, an
//! EndpointSlice, the older Endpoints, an Ingress with its class, and a NetworkPolicy.

use k8s_openapi::api::core::v1::{Endpoints, Service};
use k8s_openapi::api::discovery::v1::EndpointSlice;
use k8s_openapi::api::networking::v1::{Ingress, IngressClass, NetworkPolicy};
use serde_json::json;

use super::{and, follow, shows_part};
use crate::data::{changed_object, of_kinds, pods, scene};
use crate::fake::until;
use crate::kinds::scene::{live, nodes};
use crate::kinds::{in_panel, missing};
use crate::workloads::scene::opened;

/// The names of the pods behind the Service `web` of the scene.
fn web_pods() -> Vec<String> {
    let names = pods().into_iter().filter_map(|pod| pod.metadata.name.clone());
    names.filter(|name| name.starts_with("web-6888b778b6-")).collect()
}

#[test]
fn a_service_shows_its_addresses_ports_and_endpoints_and_an_endpoint_links_its_pod() {
    let mut window = opened::<Service>(vec![live("ams-prod", &scene::<Service>())], "web");
    let texts = ["Cluster IPs", "IP families", "Selector", "app=web", "Session affinity", "Traffic policy"];
    let texts = [&texts[..], &["PORTS", "http 80/TCP", "ENDPOINTS", "ENDPOINTSLICES", "ready"]].concat();
    assert_eq!(missing(&window, &texts), Vec::<String>::new());
    let node = nodes()[0].metadata.name.clone().expect("a node");
    let pods = web_pods();
    assert!(pods.iter().chain([&node]).all(|name| in_panel(&window, name)), "the pod and the node of each endpoint");

    let pod = pods.first().expect("a pod");
    follow(&mut window, pod);
    until(&mut window, "the panel of the pod", |window| in_panel(window, "CONTAINERS") && in_panel(window, pod));
}

#[test]
fn a_load_balancer_with_no_address_and_a_service_with_no_endpoint_tell_so() {
    let window = opened::<Service>(vec![live("ams-prod", &scene::<Service>())], "public");
    assert!(shows_part(&window, "pending: the load balancer has no address yet"));
    assert!(in_panel(&window, "31472"), "the node port");
    let window = opened::<Service>(vec![live("ams-prod", &scene::<Service>())], "nobody");
    assert_eq!(missing(&window, &["ENDPOINTS", "None."]), Vec::<String>::new());
}

#[test]
fn an_endpoint_slice_names_its_service_and_each_endpoint_with_its_state() {
    let slices = scene::<EndpointSlice>();
    let mixed = slices.iter().find(|slice| slice.metadata.name.as_deref() == Some("mixed-dsjfl")).expect("mixed");
    let window = opened::<EndpointSlice>(vec![live("ams-prod", &slices)], "mixed-dsjfl");
    let texts = ["Address type", "IPv4", "Managed by", "endpointslice-controller.k8s.io", "ENDPOINTS", "PORTS"];
    assert_eq!(missing(&window, &texts), Vec::<String>::new());
    // The slice of `mixed` has one endpoint that is ready, one that is not, and one that ends.
    assert_eq!(missing(&window, &["ready", "not ready"]), Vec::<String>::new());
    assert!(shows_part(&window, "terminating"), "{:?}", mixed.endpoints);

    let services = and(live("ams-prod", &slices), &scene::<Service>());
    let mut window = opened::<EndpointSlice>(vec![services], "web-5qcnf");
    follow(&mut window, "web");
    until(&mut window, "the panel of the Service", |window| in_panel(window, "Session affinity"));
}

#[test]
fn the_older_endpoints_show_each_address_with_its_state_target_node_and_ports() {
    let endpoints = of_kinds::<Endpoints>();
    let web = endpoints.iter().find(|object| object.metadata.name.as_deref() == Some("web")).expect("web");
    let waiting = changed_object(&**web, |value| {
        value["subsets"][0]["notReadyAddresses"] = json!([{"ip": "10.42.0.9", "nodeName": "311dd79317f1"}]);
    });
    let others = endpoints.iter().filter(|object| object.metadata.name.as_deref() != Some("web")).cloned();
    let window = opened::<Endpoints>(vec![live("ams-prod", &others.chain([waiting]).collect::<Vec<_>>())], "web");
    let texts = ["ADDRESSES", "10.42.0.4", "ready", "web-6cdb59bc46-sglsw", "311dd79317f1", "10.42.0.9", "not ready"];
    assert_eq!(missing(&window, &texts), Vec::<String>::new());
    assert!(shows_part(&window, "https 443/TCP, http 8080/TCP"), "the ports of the subset");
}

#[test]
fn an_ingress_shows_its_rules_backends_and_tls_and_its_class_links_its_panel() {
    let cluster = and(live("ams-prod", &of_kinds::<Ingress>()), &of_kinds::<IngressClass>());
    let mut window = opened::<Ingress>(vec![cluster], "web");
    let texts = ["Class", "fresnel", "RULES", "web.example.com", "/", "Prefix", "web:80", "web:https", "*", "/status"];
    let texts = [&texts[..], &["Exact", "TLS", "web-tls"]].concat();
    assert_eq!(missing(&window, &texts), Vec::<String>::new());
    assert!(shows_part(&window, "none yet: the controller gave no address"));

    follow(&mut window, "fresnel");
    until(&mut window, "the panel of the IngressClass", |window| in_panel(window, "fresnel.test/ingress"));
    assert!(shows_part(&window, "yes: an Ingress with no class gets this one"));
    assert!(shows_part(&window, "Settings shared (fresnel.test)"));
}

#[test]
fn a_network_policy_tells_its_rules_in_words() {
    let policies = of_kinds::<NetworkPolicy>();
    let web = changed_object(&*policies[0], |value| {
        value["spec"]["ingress"] = json!([{
            "from": [{"podSelector": {"matchLabels": {"app": "front"}}}, {"ipBlock": {"cidr": "10.0.0.0/8"}}],
            "ports": [{"port": 80}, {"protocol": "UDP", "port": 53}],
        }]);
    });
    let window = opened::<NetworkPolicy>(vec![live("ams-prod", &[web])], "web");
    let texts = ["Selects", "pods app=web", "Policy types", "Ingress, Egress", "INCOMING TRAFFIC", "Rule 1"];
    assert_eq!(missing(&window, &texts), Vec::<String>::new());
    assert!(shows_part(&window, "From pods app=front of the namespace of the policy; IP block 10.0.0.0/8"));
    assert!(shows_part(&window, "Ports: TCP 80, UDP 53"));
    assert!(shows_part(&window, "No rule: the selected pods take no connection of this way."), "no egress rule");
}
