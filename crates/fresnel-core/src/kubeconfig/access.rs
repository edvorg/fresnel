//! What a kubeconfig gives for the connection to one cluster, with each entry checked.
//!
//! An entry that the app cannot use is an error with its reason. The app never connects with
//! less than the kubeconfig asks for, for example without a static token.

use std::fmt::{self, Display};
use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use fresnel_net::credential::PluginSpec;
use http::Uri;
use serde_json::{Value, json};

use super::file::{ClusterEntry, EnvEntry, ExecEntry, UserEntry};
use super::material::Material;
use crate::error::CoreError;

/// The extension of a cluster entry that holds the settings for its credential plugin.
const EXEC_EXTENSION: &str = "client.authentication.k8s.io/exec";
const HTTPS_PORT: u16 = 443;

/// Where the API server is: the parts of the `server` URL of the cluster entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Server {
    /// A name or an address, with no brackets.
    pub host: String,
    /// The port.
    pub port: u16,
    /// The path before each API path. Empty for most clusters.
    pub base_path: String,
}

impl Server {
    fn parse(url: &str) -> Result<Self, CoreError> {
        let problem = |what: &str| CoreError::invalid(format!("the server URL `{url}` {what}"));
        let uri: Uri = url.parse().map_err(|_| problem("is not a URL"))?;
        if uri.scheme_str() != Some("https") {
            return Err(problem("is not an https URL; the app does not support that"));
        }
        let host = uri.host().map(|host| host.trim_start_matches('[').trim_end_matches(']'));
        let host = host.filter(|host| !host.is_empty()).ok_or_else(|| problem("has no host"))?;
        Ok(Self {
            host: host.to_owned(),
            port: uri.port_u16().unwrap_or(HTTPS_PORT),
            base_path: uri.path().trim_end_matches('/').to_owned(),
        })
    }
}

impl Display for Server {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}{}", self.host, self.port, self.base_path)
    }
}

/// How the user of a context proves the identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthKind {
    /// A credential plugin. The text is its command, for example `doctl`.
    Plugin(String),
    /// A client certificate.
    Certificate,
    /// Nothing: the requests carry no credential.
    Anonymous,
}

/// A client certificate with its key.
#[derive(Clone, PartialEq)]
pub(crate) struct Identity {
    pub(crate) certificate: Material,
    pub(crate) key: Material,
}

/// The credential plugin of a user entry.
#[derive(Clone, PartialEq)]
struct Plugin {
    spec: PluginSpec,
    /// The kubeconfig asks the app to give the cluster data to the plugin.
    wants_cluster: bool,
    /// The settings for the plugin in the cluster entry.
    config: Option<Value>,
}

/// How the app reaches one cluster. The load reads no certificate file: a file is read at
/// each connect.
#[derive(Clone, PartialEq)]
pub struct Access {
    /// Where the API server is.
    pub server: Server,
    /// The name in the certificate of the server, if it is not the host.
    pub tls_server_name: Option<String>,
    /// The `server` URL as the kubeconfig has it, for the cluster data of a plugin.
    url: String,
    pub(crate) authority: Material,
    pub(crate) identity: Option<Identity>,
    plugin: Option<Plugin>,
}

/// One named entry of a kubeconfig with the folder of its file.
pub(super) struct Entry<'a, T> {
    pub(super) name: &'a str,
    pub(super) body: &'a T,
    pub(super) folder: &'a Path,
}

impl Access {
    pub(super) fn new(
        cluster: &Entry<'_, ClusterEntry>,
        user: Option<&Entry<'_, UserEntry>>,
    ) -> Result<Self, CoreError> {
        let (name, entry) = (cluster.name, cluster.body);
        let not_supported = |what: &str| not_supported(format!("the cluster entry `{name}` {what}"));
        let url = entry.server.clone().filter(|url| !url.is_empty());
        let url = url.ok_or_else(|| CoreError::invalid(format!("the cluster entry `{name}` has no server")))?;
        if entry.insecure_skip_tls_verify == Some(true) {
            return Err(not_supported("skips the check of the server certificate"));
        }
        if entry.proxy_url.as_deref().is_some_and(|proxy| !proxy.is_empty()) {
            return Err(not_supported("uses a proxy"));
        }
        let (data, file) = (entry.certificate_authority_data.as_deref(), entry.certificate_authority.as_deref());
        let authority = Material::of(data, file, cluster.folder, "certificate authority")?
            .ok_or_else(|| not_supported("has no certificate authority"))?;
        let (identity, plugin) = match user {
            Some(user) => (identity(user)?, plugin(user, entry)?),
            None => (None, None),
        };
        Ok(Self {
            server: Server::parse(&url)?,
            tls_server_name: entry.tls_server_name.clone().filter(|name| !name.is_empty()),
            url,
            authority,
            identity,
            plugin,
        })
    }

    /// How the user proves the identity. A plugin wins over a certificate in this text;
    /// the app uses the two when the entry has the two.
    pub fn auth(&self) -> AuthKind {
        match (&self.plugin, &self.identity) {
            (Some(plugin), _) => AuthKind::Plugin(plugin.spec.command.clone()),
            (None, Some(_)) => AuthKind::Certificate,
            (None, None) => AuthKind::Anonymous,
        }
    }

    /// The credential plugin as the kubeconfig gives it, with no cluster data.
    pub fn plugin(&self) -> Option<&PluginSpec> {
        self.plugin.as_ref().map(|plugin| &plugin.spec)
    }

    /// The plugin for a connect. `authority` is the certificate authority in PEM form: a
    /// plugin that asks for the cluster data gets it, as from `kubectl`.
    pub(crate) fn plugin_for_connect(&self, authority: &[u8]) -> Option<PluginSpec> {
        let plugin = self.plugin.as_ref()?;
        let mut spec = plugin.spec.clone();
        if plugin.wants_cluster {
            let mut cluster = json!({ "server": self.url, "certificate-authority-data": STANDARD.encode(authority) });
            if let Some(fields) = cluster.as_object_mut() {
                fields.extend(self.tls_server_name.iter().map(|name| ("tls-server-name".to_owned(), json!(name))));
                fields.extend(plugin.config.iter().map(|config| ("config".to_owned(), config.clone())));
            }
            spec.cluster = Some(cluster);
        }
        Some(spec)
    }
}

/// Shows no certificate and no key.
impl fmt::Debug for Access {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Access").field("server", &self.server).field("auth", &self.auth()).finish_non_exhaustive()
    }
}

fn not_supported(what: String) -> CoreError {
    CoreError::invalid(format!("{what}; the app does not support that yet"))
}

/// The client certificate of a user entry. The checks for the credential kinds that the app
/// does not support are here: each one would make the app act as some other user.
fn identity(user: &Entry<'_, UserEntry>) -> Result<Option<Identity>, CoreError> {
    let (name, entry) = (user.name, user.body);
    let unsupported = [
        (entry.token.is_some() || entry.token_file.is_some(), "a static token"),
        (entry.username.is_some() || entry.password.is_some(), "a password"),
        (entry.auth_provider.is_some(), "an auth provider"),
        (entry.impersonate.is_some(), "a user to act as"),
    ];
    if let Some((_, what)) = unsupported.into_iter().find(|(present, _)| *present) {
        return Err(not_supported(format!("the user entry `{name}` has {what}")));
    }
    let material = |data: &Option<String>, file: &Option<String>, what| {
        Material::of(data.as_deref(), file.as_deref(), user.folder, what)
    };
    let certificate = material(&entry.client_certificate_data, &entry.client_certificate, "client certificate")?;
    let key = material(&entry.client_key_data, &entry.client_key, "client key")?;
    let half = |has: &str, lacks: &str| CoreError::invalid(format!("the user entry `{name}` has {has} and no {lacks}"));
    match (certificate, key) {
        (Some(certificate), Some(key)) => Ok(Some(Identity { certificate, key })),
        (None, None) => Ok(None),
        (Some(_), None) => Err(half("a client certificate", "client key")),
        (None, Some(_)) => Err(half("a client key", "client certificate")),
    }
}

fn plugin(user: &Entry<'_, UserEntry>, cluster: &ClusterEntry) -> Result<Option<Plugin>, CoreError> {
    let Some(exec) = &user.body.exec else {
        return Ok(None);
    };
    let name = user.name;
    let command = exec.command.clone().filter(|command| !command.is_empty());
    let command =
        command.ok_or_else(|| CoreError::invalid(format!("the exec entry of the user `{name}` has no command")))?;
    if exec.interactive_mode.as_deref() == Some("Always") {
        let text = format!("the credential plugin of the user `{name}` needs a terminal, and the app has none");
        return Err(CoreError::invalid(text));
    }
    let extensions = cluster.extensions.iter().flatten();
    let config = extensions.filter(|extension| extension.name == EXEC_EXTENSION).find_map(|found| found.body.clone());
    Ok(Some(Plugin {
        spec: spec(command, exec, user.folder),
        wants_cluster: exec.provide_cluster_info == Some(true),
        config,
    }))
}

fn spec(command: String, exec: &ExecEntry, folder: &Path) -> PluginSpec {
    let variable = |pair: &EnvEntry| (pair.name.clone(), pair.value.clone().unwrap_or_default());
    let env = exec.env.iter().flatten().map(variable).collect();
    PluginSpec {
        command,
        args: exec.args.clone().unwrap_or_default(),
        env,
        drop_env: Vec::new(),
        api_version: exec.api_version.clone(),
        install_hint: exec.install_hint.clone(),
        cluster: None,
        // A command with a relative path starts from the folder of its kubeconfig file.
        folder: Some(folder.to_owned()),
    }
}
