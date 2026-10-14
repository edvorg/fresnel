//! The parts that the requests of the operations share: the path of an object, the field
//! manager and the three types of a patch.

use http::Request;
use http::header::{CONTENT_TYPE, HeaderValue};
use kube::core::params::{GetParams, ListParams, Patch, PatchParams, PostParams};
use serde_json::Value as Json;

use super::Target;
use crate::error::CoreError;
use crate::keys::FIELD_MANAGER;
use crate::kinds::Kind;

/// The content type of a JSON patch (RFC 6902): a list of steps, each with a path.
const JSON_PATCH: &str = "application/json-patch+json";

/// The type of a patch body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PatchType {
    /// A merge patch: each map of the body merges into the object, `null` removes a key.
    Merge,
    /// A strategic merge patch: the merge of kubectl, with the keys of lists and the
    /// directives such as `$patch`.
    Strategic,
    /// A JSON patch: a list of steps such as `replace` at a path.
    Json,
}

/// The collection of a kind: in the namespace of the key, or of the cluster for a kind
/// with no namespace. A namespaced kind with no namespace would give the path
/// `namespaces//pods`, so it is an error with no request.
pub(super) fn collection(kind: Kind, namespace: &str) -> Result<kube::core::Request, CoreError> {
    let info = kind.info();
    match (info.namespaced, namespace.is_empty()) {
        (true, false) => Ok(kube::core::Request::new(info.path_in(namespace))),
        (false, _) => Ok(kube::core::Request::new(info.path())),
        (true, true) => Err(CoreError::invalid(format!("a {} is in a namespace, and the order has none", info.name))),
    }
}

/// The GET of the object of a target.
pub(super) fn get(target: &Target) -> Result<Request<Vec<u8>>, CoreError> {
    get_named(target.kind, &target.key.namespace, &target.key.name)
}

/// The GET of one object of a kind.
pub(super) fn get_named(kind: Kind, namespace: &str, name: &str) -> Result<Request<Vec<u8>>, CoreError> {
    built("the read", collection(kind, namespace)?.get(name, &GetParams::default()))
}

/// The list of a kind in a namespace with a label selector, for example the ReplicaSets of
/// a Deployment.
pub(super) fn list(kind: Kind, namespace: &str, selector: &str) -> Result<Request<Vec<u8>>, CoreError> {
    let params = ListParams::default().labels(selector);
    built("the list", collection(kind, namespace)?.list(&params))
}

/// A patch of the object of a target, or of its subresource, with the field manager.
pub(super) fn patch(
    target: &Target,
    subresource: Option<&str>,
    kind: PatchType,
    body: &Json,
) -> Result<Request<Vec<u8>>, CoreError> {
    let params = PatchParams { field_manager: Some(FIELD_MANAGER.to_owned()), ..PatchParams::default() };
    let collection = collection(target.kind, &target.key.namespace)?;
    let name = &target.key.name;
    // kube builds a JSON patch only with a crate of its own. The body is already JSON, so
    // the request of a merge patch with the content type of a JSON patch is the same
    // request, and the app needs no more crates.
    let patch = match kind {
        PatchType::Strategic => Patch::Strategic(body),
        PatchType::Merge | PatchType::Json => Patch::Merge(body),
    };
    let request = match subresource {
        Some(subresource) => collection.patch_subresource(subresource, name, &params, &patch),
        None => collection.patch(name, &params, &patch),
    };
    let mut request = built("the patch", request)?;
    if kind == PatchType::Json {
        request.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static(JSON_PATCH));
    }
    Ok(request)
}

/// The create of an object of a kind in a namespace, with the field manager. A dry run
/// lets the server check the object, and the server stores nothing.
pub(super) fn create(kind: Kind, namespace: &str, body: &Json, dry_run: bool) -> Result<Request<Vec<u8>>, CoreError> {
    let params = PostParams { dry_run, field_manager: Some(FIELD_MANAGER.to_owned()) };
    built("the create", collection(kind, namespace)?.create(&params, to_bytes(body)?))
}

/// The bytes of a JSON body.
pub(super) fn to_bytes(body: &Json) -> Result<Vec<u8>, CoreError> {
    serde_json::to_vec(body).map_err(|_| CoreError::invalid("the body has no JSON form"))
}

/// A request that kube built, or the error of the build. The error of kube names the
/// field, never a value of the object.
pub(super) fn built(
    what: &str,
    request: Result<Request<Vec<u8>>, kube::core::request::Error>,
) -> Result<Request<Vec<u8>>, CoreError> {
    request.map_err(|error| CoreError::invalid(format!("{what}: {error}")))
}

#[cfg(test)]
pub(super) mod tests {
    use http::Request;
    use serde_json::{Value as Json, json};

    use super::*;
    use crate::cluster::ClusterId;
    use crate::store::ObjectKey;

    /// A target in the cluster `a`.
    pub(in crate::ops) fn target(kind: Kind, namespace: &str, name: &str) -> Target {
        Target { cluster: ClusterId::new("a"), kind, key: ObjectKey::new(namespace, name), uid: "uid-1".into() }
    }

    /// The method, the URL, the content type and the JSON body of a request.
    pub(in crate::ops) fn parts(request: &Request<Vec<u8>>) -> (String, String, String, Json) {
        let content = request.headers().get(CONTENT_TYPE).and_then(|value| value.to_str().ok()).unwrap_or_default();
        let body =
            if request.body().is_empty() { Json::Null } else { serde_json::from_slice(request.body()).expect("JSON") };
        (request.method().to_string(), request.uri().to_string(), content.to_owned(), body)
    }

    #[test]
    fn a_path_has_the_group_the_namespace_and_the_name_and_a_kind_with_no_namespace_has_none() {
        let get = |kind, namespace, name| parts(&get_named(kind, namespace, name).expect("a request")).1;
        assert_eq!(get(Kind::Pod, "shop", "web-1"), "/api/v1/namespaces/shop/pods/web-1");
        assert_eq!(get(Kind::Deployment, "shop", "web"), "/apis/apps/v1/namespaces/shop/deployments/web");
        assert_eq!(get(Kind::Node, "", "pool-a1"), "/api/v1/nodes/pool-a1");
        let error = get_named(Kind::Pod, "", "web-1").expect_err("no namespace");
        assert_eq!(error.to_string(), "a Pod is in a namespace, and the order has none");
        assert!(get_named(Kind::Pod, "shop", "").is_err(), "kube refuses an empty name");
    }

    #[test]
    fn each_patch_type_has_its_content_type_and_the_field_manager() {
        let web = target(Kind::Deployment, "shop", "web");
        let body = json!([{"op": "replace", "path": "/spec/paused", "value": false}]);
        let (method, url, content, sent) = parts(&patch(&web, None, PatchType::Json, &body).expect("a request"));
        assert_eq!(method, "PATCH");
        assert_eq!(url, "/apis/apps/v1/namespaces/shop/deployments/web?&fieldManager=fresnel");
        assert_eq!((content.as_str(), sent), (JSON_PATCH, body));
        let merge = parts(&patch(&web, Some("scale"), PatchType::Merge, &json!({})).expect("a request"));
        assert_eq!(merge.1, "/apis/apps/v1/namespaces/shop/deployments/web/scale?&fieldManager=fresnel");
        assert_eq!(merge.2, "application/merge-patch+json");
        let strategic = parts(&patch(&web, None, PatchType::Strategic, &json!({})).expect("a request"));
        assert_eq!(strategic.2, "application/strategic-merge-patch+json");
    }

    #[test]
    fn a_create_and_a_list_have_their_parameters() {
        let (method, url, _, body) =
            parts(&create(Kind::Job, "shop", &json!({"kind": "Job"}), true).expect("a request"));
        assert_eq!(
            (method.as_str(), url.as_str()),
            ("POST", "/apis/batch/v1/namespaces/shop/jobs?&dryRun=All&fieldManager=fresnel")
        );
        assert_eq!(body, json!({"kind": "Job"}));
        let list = parts(&list(Kind::ReplicaSet, "shop", "app=web,tier in (a,b)").expect("a request"));
        assert_eq!(list.1, "/apis/apps/v1/namespaces/shop/replicasets?&labelSelector=app%3Dweb%2Ctier+in+%28a%2Cb%29");
    }
}
