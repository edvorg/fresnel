//! The two reviews of the authorization API. The server stores nothing for them, and each
//! user can send them. A review is a POST, so it goes through `send` of the gateway, with
//! the time limit of a write; nothing changes, so a review that fails can go again.

use http::Request;
use k8s_openapi::api::authorization::v1::{SelfSubjectAccessReview, SelfSubjectRulesReview};
use kube::core::params::PostParams;
use serde_json::{Value as Json, json};

use super::super::request;
use super::{Attributes, Decision, Rules};
use crate::cluster::Gateway;
use crate::error::CoreError;

const RULES: &str = "/apis/authorization.k8s.io/v1/selfsubjectrulesreviews";
const ACCESS: &str = "/apis/authorization.k8s.io/v1/selfsubjectaccessreviews";

/// Reads the rules of the user in a namespace.
pub async fn read_rules(gateway: &Gateway, namespace: &str) -> Result<Rules, CoreError> {
    let review: SelfSubjectRulesReview = gateway.send(rules_request(namespace)?).await.map_err(|error| error.error)?;
    Ok(Rules::of(namespace, review))
}

/// Asks the server if the user can make one request.
pub async fn check(gateway: &Gateway, attributes: &Attributes) -> Result<Decision, CoreError> {
    let review: SelfSubjectAccessReview =
        gateway.send(access_request(attributes)?).await.map_err(|error| error.error)?;
    Ok(decision(attributes, review))
}

/// The review of the rules in a namespace. The server refuses one with no namespace.
fn rules_request(namespace: &str) -> Result<Request<Vec<u8>>, CoreError> {
    if namespace.is_empty() {
        return Err(CoreError::invalid("a review of the rules needs a namespace"));
    }
    let body = json!({"apiVersion": "authorization.k8s.io/v1", "kind": "SelfSubjectRulesReview",
        "spec": {"namespace": namespace}});
    post(RULES, &body)
}

/// The review of one request. An empty name is each object, an empty namespace the
/// cluster or each namespace.
fn access_request(attributes: &Attributes) -> Result<Request<Vec<u8>>, CoreError> {
    let Attributes { verb, group, resource, subresource, namespace, name } = attributes;
    let mut wanted = json!({"verb": verb, "group": group, "resource": resource});
    for (key, value) in [("subresource", *subresource), ("namespace", &**namespace), ("name", &**name)] {
        if !value.is_empty() {
            wanted[key] = Json::from(value);
        }
    }
    let body = json!({"apiVersion": "authorization.k8s.io/v1", "kind": "SelfSubjectAccessReview",
        "spec": {"resourceAttributes": wanted}});
    post(ACCESS, &body)
}

fn post(path: &str, body: &Json) -> Result<Request<Vec<u8>>, CoreError> {
    request::built(
        "the review",
        kube::core::Request::new(path).create(&PostParams::default(), request::to_bytes(body)?),
    )
}

/// The answer of an access review. "Not allowed" without "denied" means that no
/// authorizer had an opinion: the server refuses the request then too.
fn decision(attributes: &Attributes, review: SelfSubjectAccessReview) -> Decision {
    let status = review.status.unwrap_or_default();
    if status.allowed {
        return Decision::Allowed;
    }
    let reason = status.reason.filter(|reason| !reason.is_empty());
    Decision::Denied(reason.map_or_else(
        || format!("the server does not let you {attributes}"),
        |reason| format!("the server does not let you {attributes}: {reason}"),
    ))
}

#[cfg(test)]
mod tests {
    use k8s_openapi::api::authorization::v1::SubjectAccessReviewStatus;

    use super::super::super::request::tests::parts;
    use super::*;

    fn eviction(namespace: &str) -> Attributes {
        Attributes {
            verb: "create",
            group: "",
            resource: "pods",
            subresource: "eviction",
            namespace: namespace.into(),
            name: "web-1".into(),
        }
    }

    #[test]
    fn a_review_of_the_rules_names_its_namespace() {
        let (method, url, content, body) = parts(&rules_request("shop").expect("a request"));
        assert_eq!(
            (method.as_str(), url.as_str(), content.as_str()),
            ("POST", "/apis/authorization.k8s.io/v1/selfsubjectrulesreviews?", "application/json")
        );
        let expected = json!({"apiVersion": "authorization.k8s.io/v1", "kind": "SelfSubjectRulesReview", "spec": {"namespace": "shop"}});
        assert_eq!(body, expected);
        assert!(rules_request("").is_err());
    }

    #[test]
    fn a_review_of_one_request_has_its_attributes_and_no_empty_one() {
        let (method, url, _, body) = parts(&access_request(&eviction("shop")).expect("a request"));
        assert_eq!(
            (method.as_str(), url.as_str()),
            ("POST", "/apis/authorization.k8s.io/v1/selfsubjectaccessreviews?")
        );
        let attributes = json!({"verb": "create", "group": "", "resource": "pods", "subresource": "eviction",
            "namespace": "shop", "name": "web-1"});
        let expected = json!({"apiVersion": "authorization.k8s.io/v1", "kind": "SelfSubjectAccessReview",
            "spec": {"resourceAttributes": attributes}});
        assert_eq!(body, expected);
        let node = Attributes {
            verb: "patch",
            group: "",
            resource: "nodes",
            subresource: "",
            namespace: "".into(),
            name: "".into(),
        };
        let sent = parts(&access_request(&node).expect("a request")).3;
        assert_eq!(sent["spec"]["resourceAttributes"], json!({"verb": "patch", "group": "", "resource": "nodes"}));
    }

    #[test]
    fn the_answer_of_a_review_is_a_decision_with_the_reason_of_the_server() {
        let answer = |allowed: bool, reason: Option<&str>| SelfSubjectAccessReview {
            status: Some(SubjectAccessReviewStatus {
                allowed,
                reason: reason.map(str::to_owned),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(decision(&eviction("shop"), answer(true, None)), Decision::Allowed);
        let text = "the server does not let you create pods/eviction web-1 in shop: RBAC: no binding";
        assert_eq!(decision(&eviction("shop"), answer(false, Some("RBAC: no binding"))), Decision::Denied(text.into()));
        let bare = decision(&eviction("shop"), answer(false, None));
        assert_eq!(bare, Decision::Denied("the server does not let you create pods/eviction web-1 in shop".into()));
    }
}
