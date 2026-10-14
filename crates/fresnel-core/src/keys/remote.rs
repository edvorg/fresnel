//! The calls of a key view through the gateway of its cluster, and the state of a write.

use std::sync::Arc;

use k8s_openapi::api::core::v1::{ConfigMap, Secret};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

use super::draft::Save;
use super::object::{KeyKind, KeyObject, Target};
use super::request;
use super::secret_type::NewSecret;
use crate::cluster::{ClusterId, Fate, Gateway, WriteError};
use crate::error::CoreError;
use crate::store::{ObjectKey, Written};

/// How far a save or a create is: the states of a job (plan section 7.3). A state that
/// nobody reads hides the end of a write, so the compiler refuses to drop one.
#[derive(Clone, Debug, PartialEq, Eq)]
#[must_use]
pub enum WriteState {
    /// The request is on its way.
    Pending,
    /// The server stored the write. The object of its answer.
    Done(Arc<KeyObject>),
    /// The object changed on the server after the read of the edit (HTTP 409), and the
    /// server changed nothing. The view reads the object again and shows each key that the
    /// user and the server both changed.
    Conflict(CoreError),
    /// The server refused the write, and nothing changed.
    Failed(CoreError),
    /// The request did not go to the server.
    NotSent(CoreError),
    /// The request went out, and no answer came back: the write can be in effect. The app
    /// reads the object and does not send the write again (design rule 6).
    Unknown(CoreError),
}

impl WriteState {
    /// The state of a save after its result. HTTP 409 is a conflict: the object changed
    /// after the read of the edit.
    pub fn of_save(result: Result<KeyObject, WriteError>) -> Self {
        match result {
            Err(WriteError { fate: Fate::Refused, error }) if error.status() == Some(409) => Self::Conflict(error),
            other => Self::of_write(other),
        }
    }

    /// The state of a create after its result. HTTP 409 means a Secret with the name
    /// exists, so it is a refusal like any other.
    pub fn of_create(result: Result<KeyObject, WriteError>) -> Self {
        Self::of_write(result)
    }

    fn of_write(result: Result<KeyObject, WriteError>) -> Self {
        match result {
            Ok(object) => Self::Done(Arc::new(object)),
            Err(WriteError { fate: Fate::Refused, error }) => Self::Failed(error),
            Err(WriteError { fate: Fate::NotSent, error }) => Self::NotSent(error),
            Err(WriteError { fate: Fate::Unknown, error }) => Self::Unknown(error),
        }
    }
}

impl KeyObject {
    /// The result of the write whose answer is this object, for the mark "change pending"
    /// of a table while its watch event did not arrive (plan section 9.10).
    pub fn written(&self) -> Option<Written> {
        let key = &self.target.key;
        Written::changed(&ObjectMeta {
            name: Some(key.name.to_string()),
            namespace: Some(key.namespace.to_string()).filter(|namespace| !namespace.is_empty()),
            uid: Some(self.target.uid.to_string()),
            resource_version: Some(self.version.to_string()),
            ..ObjectMeta::default()
        })
    }
}

/// Reads the object of a key view from its server, with the time limit of a GET. An answer
/// with an other name or UID is an error: a new object of the name is not the object of
/// the view.
pub async fn read(gateway: &Gateway, target: &Target) -> Result<KeyObject, CoreError> {
    same_cluster(gateway, &target.cluster)?;
    let request = request::read(target)?;
    let object = match target.kind {
        KeyKind::Secret => KeyObject::of_secret(target.cluster.clone(), gateway.get::<Secret>(request).await?)?,
        KeyKind::ConfigMap => {
            KeyObject::of_config_map(target.cluster.clone(), gateway.get::<ConfigMap>(request).await?)?
        }
    };
    same_object(target, object)
}

/// Sends a save one time. The answer is the object after the save. An answer of an other
/// object leaves the result unknown: the view must not show its values as the new values.
pub async fn save(gateway: &Gateway, save: &Save) -> Result<KeyObject, WriteError> {
    let not_sent = |error| WriteError { fate: Fate::NotSent, error };
    same_cluster(gateway, &save.target.cluster).map_err(not_sent)?;
    let request = request::save(save).map_err(not_sent)?;
    let cluster = save.target.cluster.clone();
    let object = match save.target.kind {
        KeyKind::Secret => KeyObject::of_secret(cluster, gateway.send::<Secret>(request).await?),
        KeyKind::ConfigMap => KeyObject::of_config_map(cluster, gateway.send::<ConfigMap>(request).await?),
    };
    object.and_then(|object| same_object(&save.target, object)).map_err(unknown)
}

/// Sends the create of a Secret one time. The answer is the new Secret.
pub async fn create(gateway: &Gateway, secret: &NewSecret) -> Result<KeyObject, WriteError> {
    let not_sent = |error| WriteError { fate: Fate::NotSent, error };
    same_cluster(gateway, &secret.cluster).map_err(not_sent)?;
    let request = request::create(secret).map_err(not_sent)?;
    let answer = gateway.send::<Secret>(request).await?;
    let target = Target {
        cluster: secret.cluster.clone(),
        kind: KeyKind::Secret,
        key: ObjectKey::new(&secret.namespace, &secret.name),
        uid: "".into(),
    };
    KeyObject::of_secret(secret.cluster.clone(), answer)
        .and_then(|object| same_object(&target, object))
        .map_err(unknown)
}

/// The server took the write, and its answer does not tell the result.
fn unknown(error: CoreError) -> WriteError {
    WriteError { fate: Fate::Unknown, error }
}

/// The object of an answer, when it is the object of `target`: the same name, and the same
/// UID when the target names one.
fn same_object(target: &Target, object: KeyObject) -> Result<KeyObject, CoreError> {
    if object.target.key != target.key {
        return Err(CoreError::invalid("the server answered with an other object"));
    }
    if !target.uid.is_empty() && object.target.uid != target.uid {
        return Err(CoreError::invalid("the object of this name is an other object now: it has an other UID"));
    }
    Ok(object)
}

/// A request goes to the cluster of its object only (design rule 8).
fn same_cluster(gateway: &Gateway, cluster: &ClusterId) -> Result<(), CoreError> {
    if gateway.cluster() == cluster {
        return Ok(());
    }
    Err(CoreError::invalid(format!("a request for {cluster} cannot go to {}", gateway.cluster())))
}
