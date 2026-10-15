//! Cordon, uncordon and the list of the pods on a node (research 08 sections 7.1 and 7.2).

use http::Request;
use kube::core::params::ListParams;
use serde_json::json;

use super::Target;
use super::request::{self, PatchType};
use crate::error::CoreError;
use crate::kinds::Kind;

/// The strategic patch of `kubectl cordon` (`closed`) and of `kubectl uncordon`: the field
/// `unschedulable` tells the scheduler to put no new pod on the node.
pub(super) fn cordon(target: &Target, closed: bool) -> Result<Request<Vec<u8>>, CoreError> {
    request::patch(target, None, PatchType::Strategic, &json!({"spec": {"unschedulable": closed}}))
}

/// The list of `kubectl drain`: the pods of each namespace with the node in their spec.
pub(super) fn pods_on(node: &str) -> Result<Request<Vec<u8>>, CoreError> {
    let params = ListParams::default().fields(&format!("spec.nodeName={node}"));
    request::built("the list of the pods", kube::core::Request::new(Kind::Pod.info().path()).list(&params))
}

#[cfg(test)]
mod tests {
    use super::super::request::tests::{parts, target};
    use super::*;

    #[test]
    fn a_cordon_is_a_strategic_patch_of_the_node_and_the_list_has_a_field_selector() {
        let node = target(Kind::Node, "", "pool-a1");
        let (method, url, content, body) = parts(&cordon(&node, true).expect("a request"));
        assert_eq!((method.as_str(), url.as_str()), ("PATCH", "/api/v1/nodes/pool-a1?&fieldManager=fresnel"));
        assert_eq!(
            (content.as_str(), body),
            ("application/strategic-merge-patch+json", json!({"spec": {"unschedulable": true}}))
        );
        assert_eq!(parts(&cordon(&node, false).expect("a request")).3, json!({"spec": {"unschedulable": false}}));
        let (method, url, _, _) = parts(&pods_on("pool-a1").expect("a request"));
        assert_eq!((method.as_str(), url.as_str()), ("GET", "/api/v1/pods?&fieldSelector=spec.nodeName%3Dpool-a1"));
    }
}
