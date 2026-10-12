//! One run of the exec credential plugin of a kubeconfig user, for example `doctl`.
//!
//! The app runs the plugin itself (design rule 7 names it as the only child process). Each
//! run has a time limit, an output limit and a kill of the full process group.

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use futures_util::FutureExt as _;
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use tokio::process::Command;

use super::document::{self, Credential};
use super::error::{PluginError, PluginFailure};
use crate::clock::WallClock;
use crate::env::ProcessEnv;
use crate::process::{self, End, RunLimits, tail};

/// More output than this is not an `ExecCredential`. The limit keeps a broken plugin from
/// filling the memory.
const MAX_OUTPUT_BYTES: usize = 1024 * 1024;
const DETAIL_CHARACTERS: usize = 2000;

/// The exec entry of a kubeconfig user, as `kubectl` reads it.
#[derive(Clone, Default, PartialEq)]
pub struct PluginSpec {
    /// The command: a bare name or a path.
    pub command: String,
    /// The arguments of the command.
    pub args: Vec<String>,
    /// The variables that the kubeconfig adds to the environment of the plugin.
    pub env: Vec<(String, String)>,
    /// The variables that the plugin must not get.
    pub drop_env: Vec<String>,
    /// The API version of the `ExecCredential` that the plugin speaks.
    pub api_version: Option<String>,
    /// The text of the kubeconfig for a plugin that is not installed.
    pub install_hint: Option<String>,
    /// The cluster data for the plugin, when the kubeconfig asks for it.
    pub cluster: Option<Value>,
    /// The folder of the kubeconfig file. A command with a relative path starts from it.
    pub folder: Option<PathBuf>,
}

/// Shows the command only. An `env` value of the kubeconfig can be a secret.
impl fmt::Debug for PluginSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PluginSpec").field("command", &self.command).finish_non_exhaustive()
    }
}

/// Gives a token. The credential task runs one source at a time.
pub(crate) trait TokenSource: Send + Sync + 'static {
    /// One run. It returns in `limit`, with a timeout failure if it must.
    fn run(&self, limit: Duration) -> BoxFuture<'_, Result<Credential, PluginError>>;
}

/// The plugin of one cluster with the environment of its process.
pub(crate) struct ExecPlugin {
    spec: PluginSpec,
    vars: ProcessEnv,
    clock: WallClock,
}

impl ExecPlugin {
    pub(crate) fn new(spec: PluginSpec, vars: ProcessEnv, clock: WallClock) -> Self {
        Self { spec, vars, clock }
    }

    /// The program file. The search runs again for each run, because the user can install
    /// the plugin while the app runs.
    fn locate(&self) -> Result<PathBuf, PluginError> {
        if self.spec.command.is_empty() {
            return Err(self.error(PluginFailure::NoCommand, String::new()));
        }
        self.vars.find_program(&self.spec.command, self.spec.folder.as_deref()).ok_or_else(|| {
            let searched = self.vars.path_entries().len();
            self.error(PluginFailure::NotFound { searched }, self.spec.install_hint.clone().unwrap_or_default())
        })
    }

    fn command(&self, program: PathBuf) -> Command {
        let mut command = Command::new(program);
        command
            .args(&self.spec.args)
            .env_clear()
            .envs(self.vars.iter())
            .envs(self.spec.env.iter().map(|(name, value)| (name, value)))
            .env("KUBERNETES_EXEC_INFO", self.exec_info());
        for name in &self.spec.drop_env {
            command.env_remove(name);
        }
        command
    }

    /// The input of the plugin, as `kubectl` gives it. The app is never interactive: it has
    /// no terminal for a question of the plugin.
    fn exec_info(&self) -> String {
        let mut spec = json!({ "interactive": false });
        if let (Some(cluster), Some(fields)) = (&self.spec.cluster, spec.as_object_mut()) {
            fields.insert("cluster".to_owned(), cluster.clone());
        }
        json!({ "apiVersion": self.spec.api_version, "kind": "ExecCredential", "spec": spec }).to_string()
    }

    async fn run_once(&self, limit: Duration) -> Result<Credential, PluginError> {
        let command = self.command(self.locate()?);
        let limits = RunLimits { time: limit, output_bytes: MAX_OUTPUT_BYTES };
        // The run waits for the exit of the plugin: a plugin can write its own cache after its answer.
        let output = process::run(command, limits, |_| false)
            .await
            .map_err(|error| self.error(PluginFailure::Start(error.to_string()), String::new()))?;
        let failure = match output.end {
            End::Exited(status) if !status.success() => PluginFailure::Exit(status.to_string()),
            End::Exited(_) | End::Complete => match document::parse(&output.stdout, self.clock.now()) {
                Ok(credential) => return Ok(credential),
                Err(failure) => failure,
            },
            End::TimedOut => PluginFailure::Timeout { limit, stopped: output.stopped },
            End::TooMuchOutput => PluginFailure::TooMuchOutput(MAX_OUTPUT_BYTES),
            End::Failed(error) => PluginFailure::Read(error.to_string()),
        };
        Err(self.error(failure, tail(&output.stderr, DETAIL_CHARACTERS)))
    }

    fn error(&self, failure: PluginFailure, detail: String) -> PluginError {
        PluginError::new(&self.spec.command, failure, detail)
    }
}

impl TokenSource for ExecPlugin {
    fn run(&self, limit: Duration) -> BoxFuture<'_, Result<Credential, PluginError>> {
        self.run_once(limit).boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plugin(spec: PluginSpec) -> ExecPlugin {
        let vars: ProcessEnv = [("PATH", "/usr/bin:/bin")].into_iter().collect();
        ExecPlugin::new(spec, vars, WallClock::system())
    }

    #[tokio::test]
    async fn a_plugin_that_is_not_in_path_gives_the_install_hint_as_detail() {
        let spec = PluginSpec {
            command: "no-such-credential-plugin".to_owned(),
            install_hint: Some("Install it with `brew install doctl`.".to_owned()),
            ..PluginSpec::default()
        };
        let error = plugin(spec).run(Duration::from_secs(1)).await.expect_err("no such program");
        assert_eq!(
            error.to_string(),
            "credential plugin `no-such-credential-plugin`: not found in the 2 folders of PATH"
        );
        assert_eq!(error.detail(), "Install it with `brew install doctl`.");

        let error = plugin(PluginSpec::default()).run(Duration::from_secs(1)).await.expect_err("no command");
        assert_eq!(error.failure(), &PluginFailure::NoCommand);
    }

    #[test]
    fn the_plugin_gets_the_exec_info_of_kubectl_and_the_debug_text_hides_the_env_values() {
        let spec = PluginSpec {
            command: "doctl".to_owned(),
            api_version: Some("client.authentication.k8s.io/v1beta1".to_owned()),
            env: vec![("DIGITALOCEAN_ACCESS_TOKEN".to_owned(), "secret-of-the-kubeconfig".to_owned())],
            cluster: Some(json!({ "server": "https://example.test" })),
            ..PluginSpec::default()
        };
        assert_eq!(format!("{spec:?}"), r#"PluginSpec { command: "doctl", .. }"#);
        let info: Value = serde_json::from_str(&plugin(spec).exec_info()).expect("JSON");
        let expected = json!({
            "apiVersion": "client.authentication.k8s.io/v1beta1",
            "kind": "ExecCredential",
            "spec": { "interactive": false, "cluster": { "server": "https://example.test" } },
        });
        assert_eq!(info, expected);
    }
}
