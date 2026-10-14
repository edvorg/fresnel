//! The rules of the user in one namespace, and their match with the rule of RBAC
//! (`pkg/apis/rbac/v1/evaluation_helpers.go` of Kubernetes 1.33).

use std::sync::Arc;

use k8s_openapi::api::authorization::v1::{ResourceRule, SelfSubjectRulesReview};

use super::{Attributes, Decision};
use crate::error::CoreError;

/// The answer of a SelfSubjectRulesReview for one namespace.
#[derive(Clone, Debug, PartialEq)]
pub struct Rules {
    namespace: Arc<str>,
    rules: Vec<ResourceRule>,
    /// True when an authorizer of the server cannot list its rules: the rules are a part
    /// of the truth, and an action with no rule can be permitted.
    incomplete: bool,
}

impl Rules {
    /// The rules of a review of `namespace`. An answer with no status has no rules: with
    /// none, each action would be denied, so it is an error.
    pub fn of(namespace: &str, review: SelfSubjectRulesReview) -> Result<Self, CoreError> {
        let status = review.status.ok_or_else(|| CoreError::invalid("the review of the rules has no status"))?;
        // An authorizer that failed can have rules that the list does not have.
        let failed = status.evaluation_error.is_some_and(|error| !error.is_empty());
        Ok(Self { namespace: namespace.into(), rules: status.resource_rules, incomplete: status.incomplete || failed })
    }

    /// The namespace of the rules.
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// True when the server gave only a part of the rules.
    pub fn is_incomplete(&self) -> bool {
        self.incomplete
    }

    /// The decision for one request in the namespace of the rules.
    ///
    /// The rules of a namespace also hold the rules of the cluster roles of the user, but
    /// they do not tell which of them work on the objects of the cluster. Thus a kind with
    /// no namespace is unknown, as each namespace other than this one.
    pub fn decide(&self, request: &Attributes) -> Decision {
        if request.namespace.is_empty() {
            return Decision::Unknown("the rules of a namespace do not tell an action on the cluster".to_owned());
        }
        if request.namespace != self.namespace {
            return Decision::Unknown(format!("the rules are of the namespace {}", self.namespace));
        }
        if self.rules.iter().any(|rule| permits(rule, request)) {
            return Decision::Allowed;
        }
        if self.incomplete {
            return Decision::Unknown("the server gave only a part of the rules".to_owned());
        }
        Decision::Denied(format!("no rule lets you {request}"))
    }
}

/// The match of one rule: each of the verb, the group, the resource and the name.
fn permits(rule: &ResourceRule, request: &Attributes) -> bool {
    let verbs = rule.verbs.iter().any(|verb| verb == "*" || verb == request.verb);
    let groups = rule.api_groups.iter().flatten().any(|group| group == "*" || group == request.group);
    verbs && groups && resource_matches(rule, request) && name_matches(rule, request)
}

/// `*` matches each resource and each subresource, `pods/log` one subresource, and
/// `*/scale` the subresource `scale` of each resource. `pods/*` matches nothing.
fn resource_matches(rule: &ResourceRule, request: &Attributes) -> bool {
    let combined = match request.subresource {
        "" => request.resource.to_owned(),
        subresource => format!("{}/{subresource}", request.resource),
    };
    rule.resources.iter().flatten().any(|resource| {
        resource == "*"
            || *resource == combined
            || (!request.subresource.is_empty() && resource.strip_prefix("*/") == Some(request.subresource))
    })
}

/// A rule with names matches only a request for one of them. A create and a list have no
/// name, so such a rule never permits them.
fn name_matches(rule: &ResourceRule, request: &Attributes) -> bool {
    let names = rule.resource_names.as_deref().unwrap_or_default();
    names.is_empty() || (!request.name.is_empty() && names.iter().any(|name| **name == *request.name))
}

#[cfg(test)]
mod tests {
    use k8s_openapi::api::authorization::v1::SubjectRulesReviewStatus;

    use super::*;

    fn rule(verbs: &[&str], groups: &[&str], resources: &[&str], names: &[&str]) -> ResourceRule {
        let texts = |texts: &[&str]| texts.iter().map(|text| (*text).to_owned()).collect::<Vec<_>>();
        ResourceRule {
            verbs: texts(verbs),
            api_groups: Some(texts(groups)),
            resources: Some(texts(resources)),
            resource_names: Some(texts(names)).filter(|names| !names.is_empty()),
        }
    }

    fn rules(namespace: &str, rules: Vec<ResourceRule>, incomplete: bool) -> Rules {
        let status = SubjectRulesReviewStatus { resource_rules: rules, incomplete, ..Default::default() };
        Rules::of(namespace, SelfSubjectRulesReview { status: Some(status), ..Default::default() }).expect("rules")
    }

    fn request(
        verb: &'static str,
        group: &'static str,
        resource: &'static str,
        subresource: &'static str,
        name: &str,
    ) -> Attributes {
        Attributes { verb, group, resource, subresource, namespace: "shop".into(), name: name.into() }
    }

    #[test]
    fn a_rule_matches_its_verb_group_resource_and_subresource() {
        let shop =
            rules("shop", vec![rule(&["get", "patch"], &["apps"], &["deployments", "deployments/scale"], &[])], false);
        assert_eq!(shop.decide(&request("patch", "apps", "deployments", "scale", "web")), Decision::Allowed);
        assert_eq!(shop.decide(&request("get", "apps", "deployments", "", "web")), Decision::Allowed);
        let denied = shop.decide(&request("delete", "apps", "deployments", "", "web"));
        assert_eq!(denied, Decision::Denied("no rule lets you delete deployments (apps) web in shop".into()));
        assert!(
            matches!(shop.decide(&request("get", "", "deployments", "", "web")), Decision::Denied(_)),
            "an other group"
        );
        assert!(matches!(shop.decide(&request("patch", "apps", "statefulsets", "scale", "db")), Decision::Denied(_)));
    }

    #[test]
    fn a_star_matches_each_value_and_star_slash_a_subresource_of_each_resource() {
        let shop = rules(
            "shop",
            vec![rule(&["*"], &["*"], &["*/scale"], &[]), rule(&["create"], &[""], &["pods/*"], &[])],
            false,
        );
        assert_eq!(shop.decide(&request("patch", "apps", "statefulsets", "scale", "db")), Decision::Allowed);
        assert!(matches!(shop.decide(&request("patch", "apps", "statefulsets", "", "db")), Decision::Denied(_)));
        let eviction = shop.decide(&request("create", "", "pods", "eviction", "web-1"));
        assert!(matches!(eviction, Decision::Denied(_)), "`pods/*` matches no subresource");
        let all = rules("shop", vec![rule(&["*"], &["*"], &["*"], &[])], false);
        assert_eq!(all.decide(&request("create", "", "pods", "eviction", "web-1")), Decision::Allowed);
    }

    #[test]
    fn a_rule_with_names_permits_only_a_request_for_one_of_them() {
        let shop = rules("shop", vec![rule(&["delete", "create"], &["batch"], &["jobs"], &["once"])], false);
        assert_eq!(shop.decide(&request("delete", "batch", "jobs", "", "once")), Decision::Allowed);
        assert!(matches!(shop.decide(&request("delete", "batch", "jobs", "", "other")), Decision::Denied(_)));
        assert!(matches!(shop.decide(&request("create", "batch", "jobs", "", "")), Decision::Denied(_)), "no name");
    }

    #[test]
    fn incomplete_rules_an_other_namespace_and_the_cluster_are_unknown() {
        let partial = rules("shop", vec![], true);
        let unknown = partial.decide(&request("get", "", "pods", "", "web"));
        assert_eq!(unknown, Decision::Unknown("the server gave only a part of the rules".into()));
        let mut elsewhere = request("get", "", "pods", "", "web");
        elsewhere.namespace = "other".into();
        assert!(matches!(partial.decide(&elsewhere), Decision::Unknown(_)));
        let all = rules("shop", vec![rule(&["*"], &["*"], &["*"], &[])], false);
        let mut node = request("patch", "", "nodes", "", "pool-a1");
        node.namespace = "".into();
        assert!(matches!(all.decide(&node), Decision::Unknown(_)), "a cluster role binding can be the source");
        assert_eq!(node.to_string(), "patch nodes pool-a1 in the cluster");
    }

    #[test]
    fn an_answer_with_no_status_is_an_error_and_an_evaluation_error_makes_the_rules_incomplete() {
        let error = Rules::of("shop", SelfSubjectRulesReview::default()).expect_err("no status");
        assert_eq!(error.to_string(), "the review of the rules has no status");
        let status = SubjectRulesReviewStatus {
            resource_rules: vec![rule(&["get"], &[""], &["pods"], &[])],
            evaluation_error: Some("webhook: no answer".to_owned()),
            ..Default::default()
        };
        let review = SelfSubjectRulesReview { status: Some(status), ..Default::default() };
        let partial = Rules::of("shop", review).expect("rules");
        assert!(partial.is_incomplete());
        assert_eq!(partial.decide(&request("get", "", "pods", "", "web")), Decision::Allowed);
        let unknown = partial.decide(&request("delete", "", "pods", "", "web"));
        assert_eq!(unknown, Decision::Unknown("the server gave only a part of the rules".into()), "not denied");
    }

    #[test]
    fn the_decision_of_many_requests_is_the_first_denial_or_else_the_first_unknown() {
        let (allowed, denied, unknown) =
            (Decision::Allowed, Decision::Denied("d".into()), Decision::Unknown("u".into()));
        assert_eq!(Decision::all([allowed.clone(), allowed.clone()]), Decision::Allowed);
        assert_eq!(Decision::all([unknown.clone(), denied.clone()]), denied);
        assert_eq!(Decision::all([allowed, unknown.clone()]), unknown);
        assert_eq!(Decision::all([]), Decision::Allowed);
    }
}
