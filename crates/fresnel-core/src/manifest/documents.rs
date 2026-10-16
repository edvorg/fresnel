//! The documents of a text for a create or an apply (plan section 12.9): each YAML document
//! is one object, and its create or apply is one job of the Operations tab.
//!
//! A document of `kind: List` gives its items, as kubectl reads the output of `kubectl get`.
//! The app knows the kinds of its sidebar only: a request needs the path of the kind and
//! whether it is in a namespace, and a job names its kind. A document of an other kind, or
//! with no name, is a refusal of that document, and the other documents go, as in kubectl.

use std::sync::Arc;

use serde_json::Value;

use super::check;
use crate::error::CoreError;
use crate::kinds::Kind;

/// One object of a text.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Document {
    pub kind: Kind,
    /// The version of the API in the document. The request goes to that version: the
    /// server refuses a body of an other one.
    pub version: Arc<str>,
    pub name: Arc<str>,
    /// The namespace of the document, if it has one.
    pub namespace: Option<Arc<str>>,
    /// The object as JSON text.
    pub body: Arc<str>,
}

impl std::fmt::Debug for Document {
    /// The identity and the size, never the body: a Secret has its values there, and a
    /// debug text can go to a log (plan section 12.6).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Document")
            .field("kind", &self.kind)
            .field("version", &self.version)
            .field("name", &self.name)
            .field("namespace", &self.namespace)
            .field("bytes", &self.body.len())
            .finish()
    }
}

impl Document {
    /// The namespace of the object in a target namespace, as with `kubectl --namespace`: a
    /// document with none goes to the target, and a document with an other one is refused,
    /// so no object goes to a namespace that the user did not choose. Empty for a kind with
    /// no namespace, whose namespace the server drops.
    pub fn namespace_in(&self, target: &str) -> Result<Arc<str>, String> {
        if !self.kind.info().namespaced {
            return Ok("".into());
        }
        match &self.namespace {
            Some(own) if **own != *target => {
                Err(format!("Its namespace {own} is not the target namespace {target}. Choose {own} to send it."))
            }
            _ if target.is_empty() => Err("It has no namespace, and no target namespace is chosen.".to_owned()),
            _ => Ok(target.into()),
        }
    }
}

/// What a text gives for one document: the object, or why it cannot go.
pub type Entry = Result<Document, String>;

/// The documents of a text, in their sequence. An empty document gives none. An error of
/// the parser refuses the whole text, with its place, as kubectl does.
pub fn documents(text: &str) -> Result<Vec<Entry>, CoreError> {
    let values = check::parse_documents(text).map_err(|error| {
        let place = error.location().map(|at| format!(" at line {}, column {}", at.line(), at.column()));
        CoreError::invalid(format!("the text is not valid YAML{}", place.unwrap_or_default()))
    })?;
    let mut entries = Vec::new();
    for value in values {
        let list = value.get("kind").and_then(Value::as_str) == Some("List")
            && value.get("apiVersion").and_then(Value::as_str) == Some("v1");
        match value.get("items").and_then(Value::as_array).filter(|_| list) {
            Some(items) => entries.extend(items.iter().map(document)),
            None => entries.push(document(&value)),
        }
    }
    Ok(entries)
}

/// The object of one document.
fn document(value: &Value) -> Entry {
    let text = |pointer: &str| value.pointer(pointer).and_then(Value::as_str).filter(|text| !text.is_empty());
    if !value.is_object() {
        return Err("It is not a map with a kind and metadata.".to_owned());
    }
    let api = text("/apiVersion").ok_or("It has no apiVersion.")?;
    let name = text("/kind").ok_or("It has no kind.")?;
    let (group, version) = api.rsplit_once('/').unwrap_or(("", api));
    let kind = Kind::ALL.iter().copied().find(|kind| kind.info().name == name && kind.info().group == group);
    let Some(kind) = kind else {
        let group = if group.is_empty() { "the core group" } else { group };
        return Err(format!("The app creates the kinds of its sidebar, and {name} of {group} is not one of them."));
    };
    let object = text("/metadata/name").ok_or("It has no metadata.name: the app sends named objects only.")?;
    let body = serde_json::to_string(value).map_err(|_| "It has no JSON form.".to_owned())?;
    Ok(Document {
        kind,
        version: version.into(),
        name: object.into(),
        namespace: text("/metadata/namespace").map(Arc::from),
        body: body.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_document_and_each_item_of_a_list_is_an_object_and_an_empty_document_is_none() {
        let text = "apiVersion: v1\nkind: Namespace\nmetadata:\n  name: shop\n---\n---\napiVersion: autoscaling/v1\n\
                    kind: HorizontalPodAutoscaler\nmetadata:\n  name: web\n  namespace: shop\n---\napiVersion: v1\n\
                    kind: List\nitems:\n- apiVersion: v1\n  kind: ConfigMap\n  metadata: {name: a}\n- {apiVersion: v1, kind: Secret, metadata: {name: b}}\n";
        let found = documents(text).expect("valid YAML");
        let names: Vec<(Kind, &str, &str, Option<&str>)> = found
            .iter()
            .map(|entry| entry.as_ref().expect("an object"))
            .map(|document| (document.kind, &*document.version, &*document.name, document.namespace.as_deref()))
            .collect();
        assert_eq!(
            names,
            [
                (Kind::Namespace, "v1", "shop", None),
                (Kind::HorizontalPodAutoscaler, "v1", "web", Some("shop")),
                (Kind::ConfigMap, "v1", "a", None),
                (Kind::Secret, "v1", "b", None),
            ]
        );
        let body: Value = serde_json::from_str(&found[0].as_ref().expect("an object").body).expect("JSON");
        assert_eq!(body, serde_json::json!({"apiVersion": "v1", "kind": "Namespace", "metadata": {"name": "shop"}}));
        assert!(!format!("{:?}", found[3]).contains("Secret, metadata"), "no body in a debug text");
    }

    #[test]
    fn a_document_with_no_kind_no_name_or_a_kind_of_no_sidebar_is_refused_and_bad_yaml_refuses_the_text() {
        let text = "kind: ConfigMap\n---\napiVersion: v1\nmetadata: {name: a}\n---\napiVersion: v1\nkind: ConfigMap\n\
                    metadata: {generateName: a-}\n---\napiVersion: cert-manager.io/v1\nkind: Certificate\nmetadata: {name: a}\n\
                    ---\n- a\n";
        let refusals: Vec<String> = documents(text).expect("valid YAML").into_iter().filter_map(Result::err).collect();
        assert_eq!(
            refusals,
            [
                "It has no apiVersion.",
                "It has no kind.",
                "It has no metadata.name: the app sends named objects only.",
                "The app creates the kinds of its sidebar, and Certificate of cert-manager.io is not one of them.",
                "It is not a map with a kind and metadata.",
            ]
        );
        let error = documents("kind: a\n---\nkind: [b\n").expect_err("bad YAML");
        assert!(error.to_string().starts_with("the text is not valid YAML at line 3"), "{error}");
    }

    #[test]
    fn a_document_goes_to_the_target_namespace_unless_it_names_an_other_one() {
        let of = |text: &str| documents(text).expect("valid YAML").remove(0).expect("an object");
        let config = of("apiVersion: v1\nkind: ConfigMap\nmetadata: {name: a}\n");
        assert_eq!(config.namespace_in("shop").as_deref(), Ok("shop"));
        assert!(config.namespace_in("").is_err(), "no namespace at all");
        let own = of("apiVersion: v1\nkind: ConfigMap\nmetadata: {name: a, namespace: shop}\n");
        assert_eq!(own.namespace_in("shop").as_deref(), Ok("shop"));
        let other = "Its namespace shop is not the target namespace web. Choose shop to send it.";
        assert_eq!(own.namespace_in("web"), Err(other.to_owned()));
        let node = of("apiVersion: v1\nkind: Namespace\nmetadata: {name: a, namespace: shop}\n");
        assert_eq!(node.namespace_in("web").as_deref(), Ok(""), "a kind with no namespace");
    }
}
