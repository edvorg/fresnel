//! The object of a key view: one Secret or ConfigMap as one read of the server gave it,
//! with each value.

use std::collections::BTreeMap;
use std::sync::Arc;

use k8s_openapi::api::core::v1::{ConfigMap, Secret};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

use super::secret_type::SecretType;
use super::special::{self, Special};
use super::value::Value;
use crate::cluster::ClusterId;
use crate::error::CoreError;
use crate::kinds::Kind;
use crate::store::ObjectKey;

/// A kind whose objects have keys with values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KeyKind {
    /// A Secret: each value is bytes.
    Secret,
    /// A ConfigMap: text values in `data` and binary values in `binaryData`.
    ConfigMap,
}

impl KeyKind {
    /// The kind with keys of a kind of the app. `None` for each other kind.
    pub fn of(kind: Kind) -> Option<Self> {
        match kind {
            Kind::Secret => Some(Self::Secret),
            Kind::ConfigMap => Some(Self::ConfigMap),
            _ => None,
        }
    }

    /// The kind of the app.
    pub fn kind(self) -> Kind {
        match self {
            Self::Secret => Kind::Secret,
            Self::ConfigMap => Kind::ConfigMap,
        }
    }
}

/// The map of an object that holds a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// `data`: each key of a Secret, and the text values of a ConfigMap.
    Data,
    /// `binaryData`: the values of a ConfigMap that are not UTF-8.
    BinaryData,
}

impl Place {
    /// The name of the map in the API.
    pub(super) fn field(self) -> &'static str {
        match self {
            Self::Data => "data",
            Self::BinaryData => "binaryData",
        }
    }
}

/// The object of a key view.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Target {
    /// The cluster of the object.
    pub cluster: ClusterId,
    /// Its kind.
    pub kind: KeyKind,
    /// Its namespace and name.
    pub key: ObjectKey,
    /// The UID of the object that the view shows, or empty for any object of the name. A
    /// read that finds an other object of the name fails: a new object with the name of
    /// an old one is not the object of the view.
    pub uid: Arc<str>,
}

/// One key of an object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The map that holds the key.
    pub place: Place,
    /// The value.
    pub value: Value,
}

/// One Secret or ConfigMap as one read of the server gave it: the base of an edit. Its
/// debug text has the size of each value and no byte of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyObject {
    /// The object, with the UID of the read.
    pub target: Target,
    /// The resource version of the read. A save sends it: the server then refuses the
    /// save of an object that changed after the read.
    pub version: Arc<str>,
    /// True when the server refuses each change of a value (`immutable: true`). Only a
    /// delete and a new create change such an object.
    pub immutable: bool,
    /// The type of a Secret. `None` for a ConfigMap.
    pub secret_type: Option<SecretType>,
    /// Each key with its value.
    pub entries: BTreeMap<String, Entry>,
}

impl KeyObject {
    /// The object of a Secret of the server.
    pub(super) fn of_secret(cluster: ClusterId, secret: Secret) -> Result<Self, CoreError> {
        let target = |key, uid| Target { cluster, kind: KeyKind::Secret, key, uid };
        let (key, uid, version) = identity(&secret.metadata)?;
        // The server returns no `stringData`: it merges that map into `data` at a write.
        let entries = secret.data.unwrap_or_default().into_iter();
        Ok(Self {
            target: target(key, uid),
            version,
            immutable: secret.immutable == Some(true),
            secret_type: Some(SecretType::named(secret.type_.as_deref().unwrap_or_default())),
            entries: entries
                .map(|(key, bytes)| (key, Entry { place: Place::Data, value: Value::new(bytes.0) }))
                .collect(),
        })
    }

    /// The object of a ConfigMap of the server.
    pub(super) fn of_config_map(cluster: ClusterId, map: ConfigMap) -> Result<Self, CoreError> {
        let (key, uid, version) = identity(&map.metadata)?;
        let text = map.data.unwrap_or_default().into_iter();
        let mut entries: BTreeMap<String, Entry> =
            text.map(|(key, text)| (key, Entry { place: Place::Data, value: Value::new(text.into_bytes()) })).collect();
        for (name, bytes) in map.binary_data.unwrap_or_default() {
            let entry = Entry { place: Place::BinaryData, value: Value::new(bytes.0) };
            // The server refuses a key in both maps, so such an answer is not of a server.
            if entries.insert(name, entry).is_some() {
                return Err(CoreError::invalid("the ConfigMap has a key in `data` and in `binaryData`"));
            }
        }
        Ok(Self {
            target: Target { cluster, kind: KeyKind::ConfigMap, key, uid },
            version,
            immutable: map.immutable == Some(true),
            secret_type: None,
            entries,
        })
    }

    /// The value of a key.
    pub fn value(&self, key: &str) -> Option<&Value> {
        self.entries.get(key).map(|entry| &entry.value)
    }

    /// The special view of the type of a Secret: its certificates or its registries.
    pub fn special(&self) -> Special {
        let value = |key: &str| self.value(key).map(Value::bytes);
        self.secret_type.as_ref().map_or(Special::None, |kind| special::of(kind, value))
    }
}

/// The namespace, the name, the UID and the resource version of an answer of the server,
/// which has each one.
fn identity(meta: &ObjectMeta) -> Result<(ObjectKey, Arc<str>, Arc<str>), CoreError> {
    let missing = |field: &str| CoreError::invalid(format!("the answer of the server has no {field}"));
    let key = ObjectKey::of(meta).ok_or_else(|| missing("name"))?;
    let uid = meta.uid.as_deref().filter(|uid| !uid.is_empty()).ok_or_else(|| missing("UID"))?;
    let version = meta.resource_version.as_deref().filter(|version| !version.is_empty());
    Ok((key, uid.into(), version.ok_or_else(|| missing("resource version"))?.into()))
}

#[cfg(test)]
mod tests;
