//! The create and the apply of a document of a text (plan section 12.9).
//!
//! A create is the POST of `kubectl create`. An apply is a server-side apply with the field
//! manager `fresnel`: the server merges the fields of the document into the object, or
//! makes it, and refuses a field that an other manager set with HTTP 409 and the list of
//! those fields. A force takes them. Both go to the API version of the document and check
//! each field strictly, as kubectl does: the server refuses a field that it does not know.

use http::Request;
use http::Uri;
use kube::core::params::{Patch, PatchParams, PostParams, ValidationDirective};
use serde_json::Value as Json;

use super::Target;
use super::request::{built, to_bytes};
use crate::error::CoreError;
use crate::keys::FIELD_MANAGER;
use crate::manifest::documents::Document;

/// The create of the object of a document in the namespace of its target.
pub(super) fn create(target: &Target, document: &Document) -> Result<Request<Vec<u8>>, CoreError> {
    let params = PostParams { dry_run: false, field_manager: Some(FIELD_MANAGER.to_owned()) };
    let body = to_bytes(&body(document)?)?;
    let request = built("the create", collection(target, document)?.create(&params, body))?;
    strict(request)
}

/// The server-side apply of a document to the object of its target.
pub(super) fn apply(target: &Target, document: &Document, force: bool) -> Result<Request<Vec<u8>>, CoreError> {
    let params = PatchParams {
        dry_run: false,
        force,
        field_manager: Some(FIELD_MANAGER.to_owned()),
        field_validation: Some(ValidationDirective::Strict),
    };
    let body = body(document)?;
    built("the apply", collection(target, document)?.patch(&target.key.name, &params, &Patch::Apply(&body)))
}

/// The JSON of a document.
fn body(document: &Document) -> Result<Json, CoreError> {
    serde_json::from_str(&document.body).map_err(|_| CoreError::invalid("the document has no JSON form"))
}

/// The collection of the kind of a document in the version of the document.
fn collection(target: &Target, document: &Document) -> Result<kube::core::Request, CoreError> {
    let info = target.kind.info();
    let group = match info.group {
        "" => format!("/api/{}", document.version),
        group => format!("/apis/{group}/{}", document.version),
    };
    let namespace = &target.key.namespace;
    match (info.namespaced, namespace.is_empty()) {
        (false, _) => Ok(kube::core::Request::new(format!("{group}/{}", info.plural))),
        (true, false) => Ok(kube::core::Request::new(format!("{group}/namespaces/{namespace}/{}", info.plural))),
        (true, true) => Err(CoreError::invalid(format!("a {} is in a namespace, and the order has none", info.name))),
    }
}

/// The request with the strict check of the fields, which a create of kube does not set.
fn strict(mut request: Request<Vec<u8>>) -> Result<Request<Vec<u8>>, CoreError> {
    let uri = format!("{}&fieldValidation=Strict", request.uri());
    *request.uri_mut() = uri.parse::<Uri>().map_err(|_| CoreError::invalid("the create has no valid URL"))?;
    Ok(request)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::kinds::Kind;
    use crate::manifest::documents;
    use crate::ops::request::tests::{parts, target};

    fn document(text: &str) -> Document {
        documents::documents(text).expect("valid YAML").remove(0).expect("an object")
    }

    #[test]
    fn a_create_goes_to_the_version_of_the_document_with_the_field_manager_and_the_strict_check() {
        let hpa = document("apiVersion: autoscaling/v1\nkind: HorizontalPodAutoscaler\nmetadata: {name: web}\n");
        let (method, url, _, body) =
            parts(&create(&target(Kind::HorizontalPodAutoscaler, "shop", "web"), &hpa).expect("a request"));
        assert_eq!(
            (method.as_str(), url.as_str()),
            (
                "POST",
                "/apis/autoscaling/v1/namespaces/shop/horizontalpodautoscalers?&fieldManager=fresnel&fieldValidation=Strict"
            )
        );
        let metadata = json!({"name": "web"});
        assert_eq!(
            body,
            json!({"apiVersion": "autoscaling/v1", "kind": "HorizontalPodAutoscaler", "metadata": metadata})
        );
        let namespace = document("apiVersion: v1\nkind: Namespace\nmetadata: {name: shop}\n");
        let url = parts(&create(&target(Kind::Namespace, "", "shop"), &namespace).expect("a request")).1;
        assert_eq!(url, "/api/v1/namespaces?&fieldManager=fresnel&fieldValidation=Strict");
        let error = create(&target(Kind::HorizontalPodAutoscaler, "", "web"), &hpa).expect_err("no namespace");
        assert_eq!(error.to_string(), "a HorizontalPodAutoscaler is in a namespace, and the order has none");
    }

    #[test]
    fn an_apply_is_a_patch_of_server_side_apply_and_a_force_says_so() {
        let config = document("apiVersion: v1\nkind: ConfigMap\nmetadata: {name: app}\ndata: {mode: fast}\n");
        let web = target(Kind::ConfigMap, "shop", "app");
        let (method, url, content, body) = parts(&apply(&web, &config, false).expect("a request"));
        assert_eq!((method.as_str(), content.as_str()), ("PATCH", "application/apply-patch+yaml"));
        assert_eq!(url, "/api/v1/namespaces/shop/configmaps/app?&fieldManager=fresnel&fieldValidation=Strict");
        assert_eq!(body.pointer("/data/mode"), Some(&json!("fast")));
        let forced = parts(&apply(&web, &config, true).expect("a request")).1;
        assert_eq!(
            forced,
            "/api/v1/namespaces/shop/configmaps/app?&force=true&fieldManager=fresnel&fieldValidation=Strict"
        );
    }
}
