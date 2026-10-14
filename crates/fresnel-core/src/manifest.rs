//! The manifest of one object (plan section 12.9): its YAML as the server gives it now.
//! M2 shows it read-only.
//!
//! The manifest is a GET of the object through the gateway and never the copy of a store:
//! a store keeps no `managedFields`, no copy of `kubectl apply` and no value of a Secret.

use kube::core::Request;
use kube::core::params::GetParams;
use serde_json::Value;

use crate::cluster::Gateway;
use crate::error::CoreError;
use crate::kinds::Kind;
use crate::store::ObjectKey;

/// What the manifest shows of `managedFields`: which client wrote which field. They are
/// long and of little use, so a manifest leaves them out until the user asks for them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ManagedFields {
    /// The manifest has no `managedFields`.
    #[default]
    Removed,
    /// The manifest has the `managedFields` of the server.
    Kept,
}

/// The manifest of one object at one moment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    /// The YAML of the object, with the keys of each map in the sequence of the alphabet,
    /// as kubectl writes them.
    pub text: String,
    /// The resource version of the object that the text shows.
    pub version: Option<String>,
    /// What the text has of `managedFields`.
    pub managed_fields: ManagedFields,
}

/// Reads one object from the server and writes it as YAML. The read has the time limit of
/// a GET. `uid` is the UID of the object that the view shows, or empty for any object of
/// the name.
///
/// A key with no namespace for a kind in a namespace is an error with no request: its path
/// would have an empty part. An answer with an other name or an other UID is an error too,
/// so that no other object shows as the manifest of this one: a StatefulSet makes a new pod
/// with the name of the old one during the read.
pub async fn fetch(
    gateway: &Gateway,
    kind: Kind,
    key: &ObjectKey,
    uid: &str,
    managed_fields: ManagedFields,
) -> Result<Manifest, CoreError> {
    let info = kind.info();
    let collection = match (info.namespaced, key.namespace.is_empty()) {
        (false, _) => info.path(),
        (true, false) => info.path_in(&key.namespace),
        (true, true) => {
            return Err(CoreError::invalid(format!("a {} is in a namespace, and the read has none", info.name)));
        }
    };
    let request = Request::new(collection).get(&key.name, &GetParams::default());
    let request = request.map_err(|error| CoreError::invalid(format!("the read of the manifest: {error}")))?;
    let mut object: Value = gateway.get(request).await?;
    if object.pointer("/metadata/name").and_then(Value::as_str) != Some(&*key.name) {
        return Err(CoreError::invalid("the server answered the read of the manifest with an other object"));
    }
    if !uid.is_empty() && object.pointer("/metadata/uid").and_then(Value::as_str) != Some(uid) {
        return Err(CoreError::invalid("the object of this name is an other object now: it has an other UID"));
    }
    if managed_fields == ManagedFields::Removed
        && let Some(metadata) = object.get_mut("metadata").and_then(Value::as_object_mut)
    {
        metadata.remove("managedFields");
    }
    let version = object.pointer("/metadata/resourceVersion").and_then(Value::as_str).map(str::to_owned);
    // The text of a YAML error can hold a value of the object, so the error has none.
    let text = serde_saphyr::to_string(&object).map_err(|_| CoreError::invalid("the object has no YAML form"))?;
    Ok(Manifest { text, version, managed_fields })
}
