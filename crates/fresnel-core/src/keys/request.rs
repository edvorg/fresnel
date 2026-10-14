//! The requests of a key view (plan section 10): the read of one object, the merge patch
//! of a save, and the create of a Secret. Each function builds one request, and a unit test
//! asserts its URL and its body.
//!
//! A save sends each changed key and `metadata.resourceVersion`, never `stringData`: the
//! server compares the version with the stored one and refuses a save of an older read with
//! HTTP 409, and a merge patch leaves each key that it does not name as it is (research 08
//! section 8.3).

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use http::Request;
use kube::core::params::{GetParams, Patch, PatchParams, PostParams};
use serde_json::{Map, Value as Json, json};

use super::draft::Save;
use super::object::{KeyKind, Place, Target};
use super::secret_type::NewSecret;
use super::value::Value;
use crate::error::CoreError;
use crate::kinds::Kind;

/// The name of the app in `managedFields`: it tells who changed a field.
pub(crate) const FIELD_MANAGER: &str = "fresnel";

/// The GET of the object of a key view.
pub(super) fn read(target: &Target) -> Result<Request<Vec<u8>>, CoreError> {
    let request = collection(target.kind.kind(), &target.key.namespace)?.get(&target.key.name, &GetParams::default());
    request.map_err(|error| CoreError::invalid(format!("the read of the object: {error}")))
}

/// The merge patch of a save.
pub(super) fn save(save: &Save) -> Result<Request<Vec<u8>>, CoreError> {
    let params = PatchParams { field_manager: Some(FIELD_MANAGER.to_owned()), ..PatchParams::default() };
    let target = &save.target;
    let request = collection(target.kind.kind(), &target.key.namespace)?;
    let request = request.patch(&target.key.name, &params, &Patch::Merge(patch_body(save)?));
    request.map_err(|error| CoreError::invalid(format!("the save of the keys: {error}")))
}

/// The create of a Secret.
pub(super) fn create(secret: &NewSecret) -> Result<Request<Vec<u8>>, CoreError> {
    let params = PostParams { field_manager: Some(FIELD_MANAGER.to_owned()), ..PostParams::default() };
    let body =
        serde_json::to_vec(&create_body(secret)).map_err(|_| CoreError::invalid("the Secret has no JSON form"))?;
    let request = collection(Kind::Secret, &secret.namespace)?.create(&params, body);
    request.map_err(|error| CoreError::invalid(format!("the create of the Secret: {error}")))
}

/// The collection of a kind in a namespace. An empty namespace would give the path
/// `namespaces//secrets`, so it is an error with no request.
fn collection(kind: Kind, namespace: &str) -> Result<kube::core::Request, CoreError> {
    if namespace.is_empty() {
        return Err(CoreError::invalid(format!("a {} is in a namespace, and the request has none", kind.info().name)));
    }
    Ok(kube::core::Request::new(kind.info().path_in(namespace)))
}

/// `{"metadata":{"resourceVersion":…},"data":{…},"binaryData":{…}}` with each changed key,
/// and `null` for a key that goes away or that leaves its map.
fn patch_body(save: &Save) -> Result<Json, CoreError> {
    let mut maps: [(Place, Map<String, Json>); 2] = [(Place::Data, Map::new()), (Place::BinaryData, Map::new())];
    let mut put = |place: Place, key: &str, value: Json| {
        if let Some((_, map)) = maps.iter_mut().find(|(of, _)| *of == place) {
            map.insert(key.to_owned(), value);
        }
    };
    for write in &save.writes {
        if let Some((place, value)) = &write.new {
            put(*place, &write.key, encode(save.target.kind, *place, value)?);
        }
        let new_place = write.new.as_ref().map(|(place, _)| *place);
        if let Some(old) = write.old.filter(|old| Some(*old) != new_place) {
            put(old, &write.key, Json::Null);
        }
    }
    let mut body = Map::new();
    body.insert("metadata".to_owned(), json!({ "resourceVersion": &*save.version }));
    for (place, map) in maps {
        if !map.is_empty() {
            body.insert(place.field().to_owned(), Json::Object(map));
        }
    }
    Ok(Json::Object(body))
}

/// The JSON form of a value: base64 in the maps of bytes, the text in `data` of a ConfigMap.
fn encode(kind: KeyKind, place: Place, value: &Value) -> Result<Json, CoreError> {
    match (kind, place) {
        (KeyKind::Secret, _) | (KeyKind::ConfigMap, Place::BinaryData) => {
            Ok(Json::String(STANDARD.encode(value.bytes())))
        }
        (KeyKind::ConfigMap, Place::Data) => match std::str::from_utf8(value.bytes()) {
            Ok(text) => Ok(Json::String(text.to_owned())),
            Err(_) => Err(CoreError::invalid("a value in `data` of a ConfigMap must be UTF-8")),
        },
    }
}

/// The object of a create, with each value in `data`.
fn create_body(secret: &NewSecret) -> Json {
    let data: Map<String, Json> =
        secret.entries.iter().map(|(key, value)| (key.clone(), Json::String(STANDARD.encode(value.bytes())))).collect();
    let mut metadata = json!({ "name": secret.name, "namespace": secret.namespace });
    if !secret.annotations.is_empty() {
        metadata["annotations"] = json!(secret.annotations);
    }
    json!({ "apiVersion": "v1", "kind": "Secret", "metadata": metadata, "type": secret.secret_type.name(), "data": data })
}

#[cfg(test)]
mod tests;
