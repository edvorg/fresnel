//! The readers that the columns of many kinds share. A text has the form of kubectl, so a
//! cell and the output of `kubectl get` agree.

use std::borrow::Cow;
use std::collections::BTreeMap;

use k8s_openapi::api::core::v1::{PodSpec, PodTemplateSpec};
use k8s_openapi::api::rbac::v1::{RoleRef, Subject};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{LabelSelector, ObjectMeta};

use super::Value;
use super::health::Condition;

/// The spec of the pods of a template.
pub(super) fn pod_spec(template: Option<&PodTemplateSpec>) -> Option<&PodSpec> {
    template?.spec.as_ref()
}

/// The names of the containers of a pod, as kubectl prints them.
pub(super) fn containers(spec: Option<&PodSpec>) -> Value<'_> {
    Value::list(spec.into_iter().flat_map(|spec| &spec.containers).map(|container| container.name.as_str()))
}

/// The images of the containers of a pod, in the sequence of the containers.
pub(super) fn images(spec: Option<&PodSpec>) -> Value<'_> {
    Value::list(each_image(spec))
}

/// The images of each container of a pod, also of the init containers, for the search.
pub(super) fn search_images<'a>(spec: Option<&'a PodSpec>, texts: &mut Vec<&'a str>) {
    let init = spec.into_iter().flat_map(|spec| spec.init_containers.iter().flatten());
    texts.extend(each_image(spec).chain(init.filter_map(|container| container.image.as_deref())));
}

fn each_image(spec: Option<&PodSpec>) -> impl Iterator<Item = &str> {
    spec.into_iter().flat_map(|spec| &spec.containers).filter_map(|container| container.image.as_deref())
}

/// A map of labels as `key=value`, in the sequence of the keys: a node selector or the
/// selector of a Service.
pub(super) fn labels(map: Option<&BTreeMap<String, String>>) -> Value<'static> {
    let map = map.filter(|map| !map.is_empty());
    map.map_or(Value::None, |map| {
        Value::Text(Cow::Owned(map.iter().map(|(key, value)| format!("{key}={value}")).collect::<Vec<_>>().join(",")))
    })
}

/// A label selector in the form of kubectl: `app=web,tier in (a,b),!legacy`, by key.
/// None for a selector with no requirement.
pub(super) fn selector(selector: Option<&LabelSelector>) -> Value<'static> {
    let Some(selector) = selector else {
        return Value::None;
    };
    let mut parts: Vec<(&str, String)> =
        selector.match_labels.iter().flatten().map(|(key, value)| (key.as_str(), format!("{key}={value}"))).collect();
    for requirement in selector.match_expressions.iter().flatten() {
        let key = requirement.key.as_str();
        let mut values: Vec<&str> = requirement.values.iter().flatten().map(String::as_str).collect();
        values.sort_unstable();
        let values = values.join(",");
        let text = match requirement.operator.as_str() {
            "In" => format!("{key} in ({values})"),
            "NotIn" => format!("{key} notin ({values})"),
            "Exists" => key.to_owned(),
            "DoesNotExist" => format!("!{key}"),
            other => format!("{key} {other} ({values})"),
        };
        parts.push((key, text));
    }
    parts.sort_by(|a, b| a.0.cmp(b.0));
    if parts.is_empty() {
        return Value::None;
    }
    Value::Text(Cow::Owned(parts.into_iter().map(|(_, text)| text).collect::<Vec<_>>().join(",")))
}

/// The names of the conditions with the status `True`, in their sequence.
pub(super) fn true_conditions<C: Condition>(conditions: Option<&Vec<C>>) -> Value<'_> {
    let names = conditions.into_iter().flatten().filter(|condition| condition.status() == "True");
    Value::list(names.map(Condition::name))
}

/// The role of a binding, as `Kind/name`.
pub(super) fn role(role: &RoleRef) -> Value<'static> {
    Value::Text(Cow::Owned(format!("{}/{}", role.kind, role.name)))
}

/// The subjects of a binding of one kind: `User`, `Group` or `ServiceAccount`. A
/// ServiceAccount shows as `namespace/name`, as in kubectl.
pub(super) fn subjects<'a>(subjects: Option<&'a Vec<Subject>>, kind: &str) -> Value<'a> {
    let of_kind: Vec<&Subject> = subjects.into_iter().flatten().filter(|subject| subject.kind == kind).collect();
    let text = |subject: &&'a Subject| match &subject.namespace {
        Some(namespace) if kind == "ServiceAccount" => Cow::Owned(format!("{namespace}/{}", subject.name)),
        _ => Cow::Borrowed(subject.name.as_str()),
    };
    if of_kind.is_empty() { Value::None } else { Value::List(of_kind.iter().map(text).collect()) }
}

/// The owner that controls the object, as `Kind/name`.
pub(super) fn controller(meta: &ObjectMeta) -> Value<'static> {
    let owners = meta.owner_references.iter().flatten();
    let controller = owners.into_iter().find(|owner| owner.controller == Some(true));
    controller.map_or(Value::None, |owner| Value::Text(Cow::Owned(format!("{}/{}", owner.kind, owner.name))))
}

/// A count of items as the number of a cell.
pub(super) fn number_of(count: usize) -> i64 {
    i64::try_from(count).unwrap_or(i64::MAX)
}

/// A whole number of the API, as a count.
pub(super) fn count(number: Option<i32>) -> Value<'static> {
    Value::number(number)
}

/// A whole number of the API that the API reads as 0 when it is absent, for example a
/// count of replicas in a status.
pub(super) fn count_or_zero(number: Option<i32>) -> Value<'static> {
    Value::Number(number.unwrap_or(0).into())
}

#[cfg(test)]
mod tests {
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelectorRequirement;

    use super::*;

    fn text(value: Value<'_>) -> String {
        match value {
            Value::Text(text) => text.into_owned(),
            Value::None => "<none>".to_owned(),
            other => panic!("not a text: {other:?}"),
        }
    }

    #[test]
    fn a_selector_has_the_form_of_kubectl_by_key() {
        let requirement = |key: &str, operator: &str, values: &[&str]| LabelSelectorRequirement {
            key: key.to_owned(),
            operator: operator.to_owned(),
            values: Some(values.iter().map(|value| (*value).to_owned()).collect()),
        };
        let selector_of = LabelSelector {
            match_labels: Some([("tier".to_owned(), "web".to_owned()), ("app".to_owned(), "shop".to_owned())].into()),
            match_expressions: Some(vec![
                requirement("zone", "In", &["b", "a"]),
                requirement("legacy", "DoesNotExist", &[]),
                requirement("canary", "Exists", &[]),
                requirement("env", "NotIn", &["dev"]),
            ]),
        };
        assert_eq!(
            text(selector(Some(&selector_of))),
            "app=shop,canary,env notin (dev),!legacy,tier=web,zone in (a,b)"
        );
        assert_eq!(text(selector(Some(&LabelSelector::default()))), "<none>", "no requirement");
        assert_eq!(text(selector(None)), "<none>");
        assert_eq!(text(labels(Some(&[("b".into(), "2".into()), ("a".into(), "1".into())].into()))), "a=1,b=2");
        assert_eq!(text(labels(Some(&BTreeMap::new()))), "<none>");
    }
}
