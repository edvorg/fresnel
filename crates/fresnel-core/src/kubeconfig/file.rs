//! The format of a kubeconfig file, as `kubectl` reads it.
//!
//! The types hold only the fields that the app uses, and serde skips each other field. No
//! type has `Debug`, because a user entry holds a key. A static token and a password never
//! go into memory: the app only reads that they are present.

use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::Value;

/// One document of a file.
#[derive(Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(super) struct File {
    pub(super) clusters: Option<Vec<Named<ClusterEntry>>>,
    pub(super) users: Option<Vec<Named<UserEntry>>>,
    pub(super) contexts: Option<Vec<Named<ContextEntry>>>,
    pub(super) current_context: Option<String>,
}

/// One entry of a list: its name, and its body below the key of the list type.
#[derive(Deserialize)]
pub(super) struct Named<T> {
    pub(super) name: String,
    #[serde(alias = "cluster", alias = "user", alias = "context", alias = "extension")]
    pub(super) body: Option<T>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(super) struct ClusterEntry {
    pub(super) server: Option<String>,
    pub(super) certificate_authority: Option<String>,
    pub(super) certificate_authority_data: Option<String>,
    pub(super) insecure_skip_tls_verify: Option<bool>,
    pub(super) tls_server_name: Option<String>,
    pub(super) proxy_url: Option<String>,
    pub(super) extensions: Option<Vec<Named<Value>>>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(super) struct UserEntry {
    pub(super) client_certificate: Option<String>,
    pub(super) client_certificate_data: Option<String>,
    pub(super) client_key: Option<String>,
    pub(super) client_key_data: Option<String>,
    pub(super) token: Option<IgnoredAny>,
    #[serde(rename = "tokenFile")]
    pub(super) token_file: Option<IgnoredAny>,
    pub(super) username: Option<IgnoredAny>,
    pub(super) password: Option<IgnoredAny>,
    pub(super) auth_provider: Option<IgnoredAny>,
    #[serde(rename = "as")]
    pub(super) impersonate: Option<IgnoredAny>,
    pub(super) exec: Option<ExecEntry>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ExecEntry {
    pub(super) api_version: Option<String>,
    pub(super) command: Option<String>,
    pub(super) args: Option<Vec<String>>,
    pub(super) env: Option<Vec<EnvEntry>>,
    pub(super) install_hint: Option<String>,
    pub(super) provide_cluster_info: Option<bool>,
    pub(super) interactive_mode: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct EnvEntry {
    pub(super) name: String,
    /// `None` for an entry with no value. `kubectl` reads it as an empty value.
    pub(super) value: Option<String>,
}

#[derive(Default, Deserialize)]
pub(super) struct ContextEntry {
    pub(super) cluster: Option<String>,
    pub(super) user: Option<String>,
    pub(super) namespace: Option<String>,
}

/// Where the YAML reader stopped.
pub(super) struct Position {
    pub(super) line: u64,
    pub(super) column: u64,
}

/// Reads each document of the text of one file.
///
/// The error is the position only. The text of the YAML reader shows a part of the file,
/// and that part can hold a key.
pub(super) fn parse(text: &str) -> Result<Vec<File>, Position> {
    let documents: Vec<Option<File>> = serde_saphyr::from_multiple(text).map_err(|error| {
        let location = error.location();
        Position {
            line: location.as_ref().map_or(0, serde_saphyr::Location::line),
            column: location.as_ref().map_or(0, serde_saphyr::Location::column),
        }
    })?;
    Ok(documents.into_iter().flatten().collect())
}
