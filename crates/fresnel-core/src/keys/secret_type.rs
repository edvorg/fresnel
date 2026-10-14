//! The types of a Secret (research 08 section 8.4), and a new Secret with its type.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::name::key_problem;
use super::special;
use super::value::Value;
use crate::cluster::ClusterId;

/// The type of a Secret. The server keeps the type of a Secret from its create on, and it
/// checks the keys that the type requires.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum SecretType {
    /// `Opaque`: any keys. The server gives this type to a Secret with none.
    Opaque,
    /// `kubernetes.io/tls`: a certificate chain and its key.
    Tls,
    /// `kubernetes.io/dockerconfigjson`: the logins of image registries.
    DockerConfigJson,
    /// `kubernetes.io/dockercfg`: the older form of the registry logins.
    DockerCfg,
    /// `kubernetes.io/basic-auth`: a user name and a password.
    BasicAuth,
    /// `kubernetes.io/ssh-auth`: a private SSH key.
    SshAuth,
    /// `kubernetes.io/service-account-token`: a token that the server makes for a
    /// ServiceAccount, which the annotation `kubernetes.io/service-account.name` names.
    ServiceAccountToken,
    /// `bootstrap.kubernetes.io/token`: a token for a node that joins the cluster.
    BootstrapToken,
    /// A type of some other tool, for example `helm.sh/release.v1`.
    Other(Arc<str>),
}

/// The annotation that names the ServiceAccount of a token Secret.
pub const SERVICE_ACCOUNT_ANNOTATION: &str = "kubernetes.io/service-account.name";

impl SecretType {
    /// The types that the dialog of a new Secret offers, in the sequence of its menu.
    pub const KNOWN: [Self; 8] = [
        Self::Opaque,
        Self::Tls,
        Self::DockerConfigJson,
        Self::DockerCfg,
        Self::BasicAuth,
        Self::SshAuth,
        Self::ServiceAccountToken,
        Self::BootstrapToken,
    ];

    /// The type with this name. A Secret with no type has the type `Opaque`.
    pub fn named(name: &str) -> Self {
        let known = Self::KNOWN.into_iter().find(|kind| kind.name() == name);
        match (known, name) {
            (Some(kind), _) => kind,
            (None, "") => Self::Opaque,
            (None, other) => Self::Other(other.into()),
        }
    }

    /// The name of the type in the API.
    pub fn name(&self) -> &str {
        match self {
            Self::Opaque => "Opaque",
            Self::Tls => "kubernetes.io/tls",
            Self::DockerConfigJson => "kubernetes.io/dockerconfigjson",
            Self::DockerCfg => "kubernetes.io/dockercfg",
            Self::BasicAuth => "kubernetes.io/basic-auth",
            Self::SshAuth => "kubernetes.io/ssh-auth",
            Self::ServiceAccountToken => "kubernetes.io/service-account-token",
            Self::BootstrapToken => "bootstrap.kubernetes.io/token",
            Self::Other(name) => name,
        }
    }

    /// The keys that a new Secret of the type starts with: the keys that the type names.
    pub fn keys(&self) -> &'static [&'static str] {
        match self {
            Self::Tls => &["tls.crt", "tls.key"],
            Self::DockerConfigJson => &[".dockerconfigjson"],
            Self::DockerCfg => &[".dockercfg"],
            Self::BasicAuth => &["username", "password"],
            Self::SshAuth => &["ssh-privatekey"],
            Self::BootstrapToken => &["token-id", "token-secret"],
            Self::Opaque | Self::ServiceAccountToken | Self::Other(_) => &[],
        }
    }
}

/// A Secret that the user makes. The app sends its values in `data`, never in `stringData`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewSecret {
    /// The cluster of the new Secret.
    pub cluster: ClusterId,
    /// Its namespace.
    pub namespace: String,
    /// Its name. A create has an explicit name: a create with a name that the server makes
    /// is not safe to send again after a timeout.
    pub name: String,
    /// Its type, which the server keeps from the create on.
    pub secret_type: SecretType,
    /// Each key with its value.
    pub entries: BTreeMap<String, Value>,
    /// Its annotations. A token Secret needs the one that names its ServiceAccount.
    pub annotations: BTreeMap<String, String>,
}

impl NewSecret {
    /// A Secret of this type with each key of the type and an empty value.
    pub fn new(cluster: ClusterId, namespace: &str, name: &str, secret_type: SecretType) -> Self {
        let entries = secret_type.keys().iter().map(|key| ((*key).to_owned(), Value::new(Vec::new()))).collect();
        let annotations = match secret_type {
            SecretType::ServiceAccountToken => [(SERVICE_ACCOUNT_ANNOTATION.to_owned(), String::new())].into(),
            _ => BTreeMap::new(),
        };
        Self { cluster, namespace: namespace.to_owned(), name: name.to_owned(), secret_type, entries, annotations }
    }

    /// What the server will refuse, as far as the app knows it. These are hints for the
    /// dialog: the server decides, and the app sends the create also with a hint.
    pub fn hints(&self) -> Vec<String> {
        let mut hints: Vec<String> = self.entries.keys().filter_map(|key| key_problem(key)).collect();
        if self.name.is_empty() || self.namespace.is_empty() {
            hints.push("a Secret needs a name and a namespace".to_owned());
        }
        let has = |key: &str| self.entries.get(key).is_some_and(|value| !value.is_empty());
        let kind = &self.secret_type;
        let account = self.annotations.get(SERVICE_ACCOUNT_ANNOTATION).is_some_and(|name| !name.is_empty());
        match kind {
            SecretType::Tls | SecretType::DockerConfigJson | SecretType::DockerCfg | SecretType::SshAuth => {
                let missing = kind.keys().iter().filter(|key| !has(key));
                hints.extend(missing.map(|key| format!("the type {} needs a value of `{key}`", kind.name())));
            }
            SecretType::BasicAuth if !has("username") && !has("password") => {
                hints.push("a basic-auth Secret needs a username or a password".to_owned());
            }
            SecretType::ServiceAccountToken if !account => {
                hints.push(format!("a token Secret needs the annotation `{SERVICE_ACCOUNT_ANNOTATION}`"));
            }
            _ => {}
        }
        let value = |key: &str| self.entries.get(key).map(Value::bytes);
        hints.extend(special::problem(kind, value));
        hints
    }
}

#[cfg(test)]
mod tests;
