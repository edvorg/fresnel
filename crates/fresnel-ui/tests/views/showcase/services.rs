//! The Services of the showcase and their EndpointSlices in the JSON of the API server: a
//! Service of each app, and the Service `kubernetes` that the API server keeps.

use k8s_openapi::api::core::v1::Pod;
use serde_json::{Value, json};

use super::objects::{hash, meta, suffix};
use super::ready;

/// The address of the Service `kubernetes`: the first of the service range of kubeadm.
const API_SERVICE_ADDRESS: &str = "10.96.0.1";

/// The EndpointSlice of a Service: an endpoint for each of these pods that has an address,
/// ready when the pod is ready, as the controller of the slices writes it.
pub(super) fn endpoint_slice(cluster: &str, service: &Value, pods: &[&Pod], age: u64) -> Value {
    let [namespace, name] = ["namespace", "name"].map(|field| service["metadata"][field].as_str().unwrap_or_default());
    let slice = format!("{name}-{}", suffix(&[cluster, namespace, name, "slice"], 5));
    let endpoint = |pod: &&Pod| {
        let address = pod.status.as_ref()?.pod_ip.clone()?;
        let ready = ready(pod);
        let target = json!({"kind": "Pod", "namespace": namespace, "name": pod.metadata.name, "uid": pod.metadata.uid});
        Some(json!({"addresses": [address], "conditions": {"ready": ready, "serving": ready, "terminating": false},
            "nodeName": pod.spec.as_ref()?.node_name, "targetRef": target}))
    };
    let mut slice = json!({
        "metadata": meta(cluster, "EndpointSlice", namespace, &slice, age),
        "addressType": "IPv4",
        "endpoints": pods.iter().filter_map(endpoint).collect::<Vec<_>>(),
        "ports": [{"name": "", "port": service["spec"]["ports"][0]["targetPort"], "protocol": "TCP"}],
    });
    slice["metadata"]["labels"] = json!({"kubernetes.io/service-name": name,
        "endpointslice.kubernetes.io/managed-by": "endpointslice-controller.k8s.io"});
    slice
}

/// A Service of an app on one port, with an address of the service range of kubeadm.
pub(super) fn service(cluster: &str, (namespace, name, app): (&str, &str, &str), port: u16, age: u64) -> Value {
    let address = hash(&[cluster, namespace, name]);
    json!({
        "metadata": meta(cluster, "Service", namespace, name, age),
        "spec": {"type": "ClusterIP", "clusterIP": format!("10.96.{}.{}", address % 64, 2 + address % 250),
            "ports": [{"port": port, "targetPort": port, "protocol": "TCP"}], "selector": {"app": app}},
    })
}

/// The Service `kubernetes` of the namespace default, with no selector, and its EndpointSlice,
/// which the API server writes itself: the address of the control plane in the network
/// `10.<network>.0.0/16` of the nodes, above the addresses of the nodes.
pub(super) fn api_service(cluster: &str, network: u8, age: u64) -> (Value, Value) {
    let mut service = json!({
        "metadata": meta(cluster, "Service", "default", "kubernetes", age),
        "spec": {"type": "ClusterIP", "clusterIP": API_SERVICE_ADDRESS,
            "ports": [{"name": "https", "port": 443, "targetPort": 6443, "protocol": "TCP"}]},
    });
    service["metadata"]["labels"] = json!({"component": "apiserver", "provider": "kubernetes"});
    let mut slice = json!({
        "metadata": meta(cluster, "EndpointSlice", "default", "kubernetes", age),
        "addressType": "IPv4",
        "endpoints": [{"addresses": [format!("10.{network}.8.2")], "conditions": {"ready": true}}],
        "ports": [{"name": "https", "port": 6443, "protocol": "TCP"}],
    });
    slice["metadata"]["labels"] = json!({"kubernetes.io/service-name": "kubernetes"});
    (service, slice)
}
