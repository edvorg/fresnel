//! The save of the text of an editor (plan section 12.9): the object of the text, and the
//! replace of the object with the resource version of its read.
//!
//! The text goes as the parser reads it, with the scalar rules of kubectl, so a hint never
//! stops a save. The values of a Secret show decoded in the text, and the save encodes them
//! again. The replace names the version of the read: when the object changed since, the
//! server refuses it with HTTP 409 and changes nothing. A dry run first lets the server check
//! the object and give it as it would store it, for the diff before the save. As in kubectl
//! edit, `fieldValidation=Strict` makes the server refuse a field that it does not know,
//! where it would drop it with no word.

use http::Uri;
use kube::core::params::PostParams;
use serde_json::Value;

use super::edit::{EditText, text_of};
use super::{check, collection, same_object, secret};
use crate::cluster::{Fate, Gateway, WriteError};
use crate::error::CoreError;
use crate::keys::FIELD_MANAGER;
use crate::kinds::Kind;
use crate::store::ObjectKey;

/// The object that a save of the text of an editor of `kind` sends: the text as the parser
/// reads it, the values of a Secret encoded again, and the resource version of the read.
/// The text must parse; the server checks the rest.
pub fn body(text: &str, kind: Kind, version: &str) -> Result<Value, CoreError> {
    let mut object = check::parse(text).map_err(|error| {
        let place = error.location().map(|at| format!(" at line {}, column {}", at.line(), at.column()));
        CoreError::invalid(format!("the text is not valid YAML{}", place.unwrap_or_default()))
    })?;
    if kind == Kind::Secret {
        secret::encode(text, &mut object);
    }
    // A replace with no version would write over each change since the read.
    if let Some(metadata) = object.get_mut("metadata").and_then(Value::as_object_mut) {
        metadata.insert("resourceVersion".to_owned(), version.into());
    }
    Ok(object)
}

/// Sends the replace of the object of `key` one time, or its dry run, after which the server
/// stores nothing. Gives the text of the answer: the object as the server stores it. `uid`
/// is the UID of the object of the editor: an answer of an other object has a result that
/// is not known.
pub async fn replace(
    gateway: &Gateway,
    (kind, key, uid): (Kind, &ObjectKey, &str),
    body: &Value,
    dry_run: bool,
) -> Result<EditText, WriteError> {
    let not_sent = |error| WriteError { fate: Fate::NotSent, error };
    let request = request(kind, key, body, dry_run).map_err(not_sent)?;
    let answer: Value = gateway.send(request).await?;
    let unknown = |error| WriteError { fate: Fate::Unknown, error };
    same_object(&answer, key, uid, "the save").map_err(unknown)?;
    text_of(answer, kind).map_err(unknown)
}

/// The PUT of a replace, with the field manager and the strict check of the fields.
fn request(kind: Kind, key: &ObjectKey, body: &Value, dry_run: bool) -> Result<http::Request<Vec<u8>>, CoreError> {
    let bytes = serde_json::to_vec(body).map_err(|_| CoreError::invalid("the object has no JSON form"))?;
    let params = PostParams { dry_run, field_manager: Some(FIELD_MANAGER.to_owned()) };
    let request = collection(kind, key, "save")?.replace(&key.name, &params, bytes);
    let mut request = request.map_err(|error| CoreError::invalid(format!("the save: {error}")))?;
    let uri = format!("{}&fieldValidation=Strict", request.uri());
    *request.uri_mut() = uri.parse::<Uri>().map_err(|_| CoreError::invalid("the save has no valid URL"))?;
    Ok(request)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn the_body_is_the_text_as_the_parser_reads_it_with_the_version_of_the_read() {
        let text = "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: app\n  namespace: shop\n  resourceVersion: \"1\"\n\
                    data:\n  mode: 0644\n";
        let body = body(text, Kind::ConfigMap, "42").expect("a body");
        let metadata = json!({"name": "app", "namespace": "shop", "resourceVersion": "42"});
        let expected = json!({"apiVersion": "v1", "kind": "ConfigMap", "metadata": metadata, "data": {"mode": 420}});
        assert_eq!(body, expected, "the octal number of kubectl, and the version of the read in place of the text");
        let error = super::body("kind: [Pod\n", Kind::Pod, "1").expect_err("no YAML");
        assert!(error.to_string().starts_with("the text is not valid YAML at line"), "{error}");
        let secret = body_of_secret("data:\n  password: hunter2\nmetadata:\n  name: db\n");
        assert_eq!(secret.pointer("/data/password"), Some(&json!("aHVudGVyMg==")), "encoded again");
    }

    fn body_of_secret(text: &str) -> Value {
        body(text, Kind::Secret, "7").expect("a body")
    }

    #[test]
    fn the_replace_and_its_dry_run_name_the_object_the_field_manager_and_the_strict_check() {
        let key = ObjectKey::new("shop", "web");
        let body = json!({"kind": "Deployment"});
        let dry = request(Kind::Deployment, &key, &body, true).expect("a request");
        assert_eq!(dry.method(), http::Method::PUT);
        let url =
            "/apis/apps/v1/namespaces/shop/deployments/web?&dryRun=All&fieldManager=fresnel&fieldValidation=Strict";
        assert_eq!(dry.uri().to_string(), url);
        assert_eq!(serde_json::from_slice::<Value>(dry.body()).expect("JSON"), body);
        let real = request(Kind::Node, &ObjectKey::new("", "pool-a1"), &body, false).expect("a request");
        assert_eq!(real.uri().to_string(), "/api/v1/nodes/pool-a1?&fieldManager=fresnel&fieldValidation=Strict");
        let error = request(Kind::Pod, &ObjectKey::new("", "web"), &body, false).expect_err("no namespace");
        assert_eq!(error.to_string(), "a Pod is in a namespace, and the save has none");
    }
}
