//! The requests of a terminal besides its stream (plan section 12.8): the pod of a node shell,
//! a debug container, and the delete of the pod at the end (research 08, sections 6.4, 7.3).

use http::Request;
use serde_json::{Value as Json, json};

use super::{Target, delete, request};
use crate::error::CoreError;
use crate::kinds::Kind;

/// The longest name of a container, a DNS label, and of the start of a pod name here.
const LABEL: usize = 63;

/// The name of a new node shell pod: `fresnel-shell-<node>-<5 letters>`, a valid pod name
/// for each node name, so the user finds the node of a pod that stays.
pub(crate) fn node_shell_name(node: &str) -> String {
    let tail = format!("-{}", suffix());
    let start: String = format!("fresnel-shell-{node}").chars().take(LABEL - tail.len()).collect();
    format!("{}{tail}", start.trim_end_matches(['-', '.']))
}

/// The name of a new debug container. A pod keeps each ephemeral container for its life, so
/// each one needs a new name.
pub(crate) fn debug_name() -> String {
    format!("fresnel-debug-{}", suffix())
}

/// The pod of a node shell, as Freelens makes it: on the node, with its process, IPC and
/// network namespaces, privileged, and with each taint tolerated. It does not start again,
/// and its delete kills its processes at once. The container takes the input of an attach
/// and has a terminal; the app deletes the pod at the end of the session.
pub(crate) fn node_shell_pod(name: &str, node: &str, image: &str, command: &[String]) -> Json {
    json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": {
            "name": name,
            "labels": {"app.kubernetes.io/managed-by": "fresnel", "app.kubernetes.io/component": "node-shell"},
        },
        "spec": {
            "nodeName": node,
            "hostPID": true,
            "hostIPC": true,
            "hostNetwork": true,
            "restartPolicy": "Never",
            "terminationGracePeriodSeconds": 0,
            "tolerations": [{"operator": "Exists"}],
            "containers": [{
                "name": "shell",
                "image": image,
                "command": command,
                "stdin": true,
                "tty": true,
                "securityContext": {"privileged": true},
            }],
        },
    })
}

/// The create of a pod in a namespace.
pub(crate) fn create_pod(namespace: &str, pod: &Json) -> Result<Request<Vec<u8>>, CoreError> {
    request::create(Kind::Pod, namespace, pod, false)
}

/// The read of a pod, after a write whose result is not known (design rule 6).
pub(crate) fn read_pod(namespace: &str, name: &str) -> Result<Request<Vec<u8>>, CoreError> {
    request::get_named(Kind::Pod, namespace, name)
}

/// A debug container as `kubectl debug` adds it: in the process namespace of `target`, with
/// the input of an attach and a terminal.
pub(crate) fn debug_container(name: &str, image: &str, target: &str, command: &[String]) -> Json {
    json!({"spec": {"ephemeralContainers": [{
        "name": name,
        "image": image,
        "command": command,
        "stdin": true,
        "tty": true,
        "targetContainerName": target,
        "terminationMessagePolicy": "File",
    }]}})
}

/// The patch that adds a debug container. A strategic merge adds an entry to the list by its
/// name and keeps the others, as kubectl does.
pub(crate) fn add_debug_container(pod: &Target, container: &Json) -> Result<Request<Vec<u8>>, CoreError> {
    request::patch(pod, Some("ephemeralcontainers"), request::PatchType::Strategic, container)
}

/// The delete of the pod of a node shell with no grace period, and with its UID, so no
/// other pod of the name goes.
pub(crate) fn delete_pod_now(pod: &Target) -> Result<Request<Vec<u8>>, CoreError> {
    delete::force_delete(pod)
}

/// Five small letters and digits.
fn suffix() -> String {
    (0..5).map(|_| fastrand::alphanumeric().to_ascii_lowercase()).collect()
}

#[cfg(test)]
mod tests {
    use super::super::request::tests::{parts, target};
    use super::*;

    #[test]
    fn a_node_shell_is_a_privileged_pod_on_its_node_with_the_namespaces_of_the_node() {
        let command = ["nsenter".to_owned(), "sh".to_owned()];
        let pod = node_shell_pod("fresnel-shell-n1-abcde", "n1", "busybox:1.36.1", &command);
        let (method, url, content, body) = parts(&create_pod("kube-system", &pod).expect("a request"));
        assert_eq!(
            (method.as_str(), url.as_str()),
            ("POST", "/api/v1/namespaces/kube-system/pods?&fieldManager=fresnel")
        );
        assert_eq!(content, "application/json");
        assert_eq!(body["spec"]["nodeName"], "n1");
        assert_eq!((body["spec"]["hostPID"].clone(), body["spec"]["hostNetwork"].clone()), (json!(true), json!(true)));
        assert_eq!(body["spec"]["tolerations"], json!([{"operator": "Exists"}]));
        let container = &body["spec"]["containers"][0];
        assert_eq!(container["securityContext"]["privileged"], true);
        assert_eq!((container["stdin"].clone(), container["tty"].clone()), (json!(true), json!(true)));
        assert_eq!(container["command"], json!(["nsenter", "sh"]));
        assert_eq!(body["metadata"]["labels"]["app.kubernetes.io/managed-by"], "fresnel");
    }

    #[test]
    fn a_debug_container_is_a_strategic_patch_of_the_subresource_with_its_target() {
        let pod = target(Kind::Pod, "shop", "web-1");
        let container = debug_container("fresnel-debug-abcde", "busybox:1.36.1", "app", &["sh".to_owned()]);
        let (method, url, content, body) = parts(&add_debug_container(&pod, &container).expect("a request"));
        assert_eq!(method, "PATCH");
        assert_eq!(url, "/api/v1/namespaces/shop/pods/web-1/ephemeralcontainers?&fieldManager=fresnel");
        assert_eq!(content, "application/strategic-merge-patch+json");
        let added = &body["spec"]["ephemeralContainers"][0];
        assert_eq!(
            (added["name"].clone(), added["targetContainerName"].clone()),
            (json!("fresnel-debug-abcde"), json!("app"))
        );
        assert_eq!((added["stdin"].clone(), added["tty"].clone()), (json!(true), json!(true)));
    }

    #[test]
    fn the_delete_of_a_node_shell_has_no_grace_period_and_the_uid_and_each_name_fits() {
        let (method, url, _, body) = parts(&delete_pod_now(&target(Kind::Pod, "kube-system", "s")).expect("a request"));
        assert_eq!((method.as_str(), url.as_str()), ("DELETE", "/api/v1/namespaces/kube-system/pods/s?"));
        assert_eq!(
            (body["gracePeriodSeconds"].clone(), body["preconditions"]["uid"].clone()),
            (json!(0), json!("uid-1"))
        );
        let long = node_shell_name(&format!("{}.example.com", "pool-".repeat(20)));
        assert!(long.len() <= LABEL && long.starts_with("fresnel-shell-pool-"), "{long}");
        assert!(!long.contains("--") && !long.contains(".-"), "{long}");
        assert_eq!(node_shell_name("n1").len(), "fresnel-shell-n1-abcde".len());
        assert!(debug_name().starts_with("fresnel-debug-") && debug_name().len() == 19);
    }
}
