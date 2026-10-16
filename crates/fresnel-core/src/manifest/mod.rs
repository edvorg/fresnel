//! The manifest of one object (plan section 12.9): its YAML as the server gives it now.
//! M2 shows it read-only, and M11 adds the editor.
//!
//! The manifest is a GET of the object through the gateway and never the copy of a store:
//! a store keeps no `managedFields`, no copy of `kubectl apply` and no value of a Secret.
//!
//! - `edit`: the text of the editor of one object. `secret`: the values of a Secret in it.
//! - `check`: the check of an edited text: the errors of the YAML parser, and hints.

pub mod check;
mod edit;
mod secret;

pub use edit::{Decoded, EditText, LINE_LIMIT, read_for_edit};

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
#[derive(Clone, PartialEq, Eq)]
pub struct Manifest {
    /// The YAML of the object, with the keys of each map in the sequence of the alphabet,
    /// as kubectl writes them.
    pub text: String,
    /// The resource version of the object that the text shows.
    pub version: Option<String>,
    /// What the text has of `managedFields`.
    pub managed_fields: ManagedFields,
}

impl std::fmt::Debug for Manifest {
    /// The size of the text, never the text: the manifest of a Secret has its values, and
    /// a debug text can go to a log (plan section 12.6).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Manifest")
            .field("bytes", &self.text.len())
            .field("version", &self.version)
            .field("managed_fields", &self.managed_fields)
            .finish()
    }
}

/// Reads one object from the server and writes it as YAML. The read has the time limit of
/// a GET. `uid` is the UID of the object that the view shows, or empty for any object of
/// the name.
pub async fn fetch(
    gateway: &Gateway,
    kind: Kind,
    key: &ObjectKey,
    uid: &str,
    managed_fields: ManagedFields,
) -> Result<Manifest, CoreError> {
    let mut object = read(gateway, kind, key, uid).await?;
    if managed_fields == ManagedFields::Removed {
        remove_managed_fields(&mut object);
    }
    let version = version(&object);
    // The text of a YAML error can hold a value of the object, so the error has none.
    let text = serde_saphyr::to_string(&object).map_err(|_| CoreError::invalid("the object has no YAML form"))?;
    Ok(Manifest { text, version, managed_fields })
}

/// Reads one object from the server as JSON, with the time limit of a GET.
///
/// A key with no namespace for a kind in a namespace is an error with no request: its path
/// would have an empty part. An answer with an other name or an other UID is an error too,
/// so that no other object shows as the manifest of this one: a StatefulSet makes a new pod
/// with the name of the old one during the read.
async fn read(gateway: &Gateway, kind: Kind, key: &ObjectKey, uid: &str) -> Result<Value, CoreError> {
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
    let object: Value = gateway.get(request).await?;
    if object.pointer("/metadata/name").and_then(Value::as_str) != Some(&*key.name) {
        return Err(CoreError::invalid("the server answered the read of the manifest with an other object"));
    }
    if !uid.is_empty() && object.pointer("/metadata/uid").and_then(Value::as_str) != Some(uid) {
        return Err(CoreError::invalid("the object of this name is an other object now: it has an other UID"));
    }
    Ok(object)
}

/// Removes which client wrote which field: a long list that most users never need.
fn remove_managed_fields(object: &mut Value) {
    if let Some(metadata) = object.get_mut("metadata").and_then(Value::as_object_mut) {
        metadata.remove("managedFields");
    }
}

/// The resource version of an object.
fn version(object: &Value) -> Option<String> {
    object.pointer("/metadata/resourceVersion").and_then(Value::as_str).map(str::to_owned)
}
