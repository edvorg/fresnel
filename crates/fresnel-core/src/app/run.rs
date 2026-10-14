//! The task of the core: its start, and then each command of the UI.
//!
//! The start reads the login shell environment and the kubeconfig files. A command can read
//! the certificate files of a cluster. A read of a file can wait for a slow disk, so it
//! runs on the threads for blocking calls. A worker thread of the runtime that waits can
//! stop the timers and the sockets of each cluster.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use fresnel_net::Cause;
use fresnel_net::detect::{Disturbance, Monitor};
use fresnel_net::env::{self, EnvSource, ProcessEnv};

use super::data::Connected;
use super::manifest::{ManifestReader, ManifestRequest};
use super::view::{ShellNote, Startup};
use super::{APP, Command, Inner, Phase};
use crate::cluster::{ClusterId, LinkState, Registry};
use crate::error::CoreError;
use crate::kinds::Kind;
use crate::kubeconfig::{self, Context};

/// One run of the task. After an error or a panic, the next run continues from the state
/// that the core has: a start that was complete does not run again.
pub(super) async fn run(inner: Arc<Inner>) -> Result<(), CoreError> {
    let registry = inner.prepare().await?;
    loop {
        // A command that arrives after this read leaves a permit, so the wait ends at once.
        while let Some(command) = inner.next_command() {
            inner.apply(&registry, command).await?;
        }
        // One write for all commands of the queue: the button "All" sends one for each cluster.
        inner.save_state(&registry).await?;
        inner.command_sent.notified().await;
    }
}

impl Inner {
    /// The start of the core. Gives the registry with the clusters of the kubeconfig files.
    async fn prepare(&self) -> Result<Registry, CoreError> {
        let startup = self.state().startup;
        if let (Startup::Ready, Some(registry)) = (startup, self.registry.get()) {
            return Ok(registry.clone());
        }
        // The settings come first: the login shell can take seconds, and the keys of the
        // user must act before that.
        self.load_config().await?;
        let env = self.environment().await;
        let registry = self.registry.get_or_init(|| {
            Registry::new(self.tasks.clone(), self.runtime.clone(), env.clone(), self.settings.net.clone())
        });
        self.enter(Startup::Kubeconfig);
        let saved = self.load_saved_state().await?;
        let current = self.load_kubeconfig(&env, registry).await?;
        self.restore_at_start(saved, current, registry);
        self.start_monitor(registry);
        self.reconcile(registry).await?;
        self.enter(Startup::Ready);
        Ok(registry.clone())
    }

    /// Runs a call that reads files on a thread for blocking calls, and waits for its result.
    pub(super) async fn off_worker<T: Send + 'static>(
        &self,
        read: impl FnOnce() -> T + Send + 'static,
    ) -> Result<T, CoreError> {
        let done = self.runtime.spawn_blocking(read).await;
        done.map_err(|error| CoreError::invalid(format!("a read of files did not end: {error}")))
    }

    fn enter(&self, startup: Startup) {
        self.state().startup = startup;
        self.repaint.request();
    }

    /// The environment of the login shell. The shell runs one time for the life of the core.
    async fn environment(&self) -> ProcessEnv {
        let known = self.state().env.clone();
        if let Some(env) = known {
            return env;
        }
        self.enter(Startup::Environment);
        let resolved = env::resolve(&self.base_env, &self.settings.env).await;
        let problem = match resolved.source {
            EnvSource::LoginShell(shell) => {
                let time = resolved.elapsed.as_millis();
                self.journal
                    .info(APP, format!("the login shell {} gave its environment in {time} ms", shell.display()));
                None
            }
            EnvSource::Fallback(problem) => {
                let note = "the app looks for a credential plugin in its own PATH and in the fixed folders";
                self.journal.warning(APP, format!("{problem}; {note}"));
                Some(ShellNote { problem: problem.to_string(), detail: problem.detail().to_owned() })
            }
        };
        let mut state = self.state();
        state.shell_problem = problem;
        state.env = Some(resolved.vars.clone());
        resolved.vars
    }

    /// Reads the kubeconfig files and gives their contexts to the registry. Returns the
    /// `current-context` of the files.
    async fn load_kubeconfig(&self, env: &ProcessEnv, registry: &Registry) -> Result<Option<String>, CoreError> {
        let sources = kubeconfig::default_sources(env);
        let loaded = self.off_worker(move || kubeconfig::load(&sources)).await?;
        let files = count(loaded.sources.len() - loaded.absent.len(), "kubeconfig file");
        let text = format!("read {} from {files}", count(loaded.contexts.len(), "context"));
        if loaded.contexts.is_empty() {
            let sources: Vec<String> = loaded.sources.iter().map(|path| path.display().to_string()).collect();
            self.journal.warning(APP, format!("{text}; the app looked for: {}", sources.join(", ")));
        } else {
            self.journal.info(APP, text);
        }
        for problem in &loaded.problems {
            self.journal.warning(APP, format!("{}: {}", problem.path.display(), problem.text));
        }
        let current = loaded.current.filter(|name| loaded.contexts.iter().any(|context| &context.name == name));
        let contexts = self.with_kept_clusters(registry, loaded.contexts, &loaded.unread);
        let state = self.state();
        // The stop emptied the registry. A load that ends after the stop must not fill it again.
        if state.phase != Phase::Stopped {
            registry.set_contexts(contexts);
        }
        Ok(current)
    }

    /// Adds the clusters that a file gave before to the contexts of a load in which the
    /// file does not load (design rule 5). Without them, a syntax error in a file would
    /// close the connections of each of its clusters.
    fn with_kept_clusters(&self, registry: &Registry, mut contexts: Vec<Context>, unread: &[PathBuf]) -> Vec<Context> {
        let before = registry.snapshot().into_iter().map(|cluster| cluster.context);
        let kept: Vec<Context> = before
            .filter(|old| unread.contains(&old.source) && contexts.iter().all(|context| context.name != old.name))
            .collect();
        if !kept.is_empty() {
            let text = format!("{} of a file that does not load now stay as they were", count(kept.len(), "cluster"));
            self.journal.warning(APP, text);
        }
        contexts.extend(kept);
        contexts
    }

    /// Starts the detectors of a wake and of a network change. Each one raises the signal
    /// of each connected cluster.
    fn start_monitor(&self, registry: &Registry) {
        let Some(detectors) = self.state().detectors.take() else {
            return;
        };
        let (registry, journal, repaint) = (registry.clone(), self.journal.clone(), self.repaint.clone());
        let report = move |disturbance: Disturbance| {
            registry.raise(disturbance.cause());
            let what = match disturbance {
                Disturbance::Wake(slept) => format!("the computer slept for {} s", slept.as_secs()),
                Disturbance::NetworkChange => "the network changed".to_owned(),
            };
            journal.info(APP, format!("{what}: each cluster tests its connections and ends its retry delays"));
            repaint.request();
        };
        let monitor = Monitor::start(&self.tasks, detectors, self.settings.detect_period, report);
        let mut state = self.state();
        // A stopped core keeps no monitor: its task would run to the end of the runtime.
        state.monitor = Some(monitor).filter(|_| state.phase != Phase::Stopped);
    }

    fn next_command(&self) -> Option<Command> {
        self.state().commands.pop_front()
    }

    async fn apply(&self, registry: &Registry, command: Command) -> Result<(), CoreError> {
        match command {
            Command::Select { cluster, selected } => self.select(cluster, selected),
            Command::RetryNow(cluster) => self.retry(registry, &cluster).await?,
            // A layout, the shown kinds, a manifest and a key view change no connection.
            Command::SaveTable { table, layout } => {
                self.save_table(table, layout);
                return Ok(());
            }
            Command::ShowKinds(kinds) => {
                self.show_kinds(kinds);
                return Ok(());
            }
            Command::ReadManifest(request) => {
                self.read_manifest(registry, request);
                return Ok(());
            }
            Command::ShowKeys(target) => {
                self.show_keys(target);
                return Ok(());
            }
            Command::SaveKeys(save) => {
                self.save_keys(save);
                return Ok(());
            }
            Command::CreateSecret(secret) => {
                self.create_secret(registry, secret);
                return Ok(());
            }
        }
        self.reconcile(registry).await
    }

    /// Holds the stores of the full objects of these kinds in each connected cluster, and
    /// in each cluster that connects later. The stores of the other kinds stop after their time.
    fn show_kinds(&self, kinds: BTreeSet<Kind>) {
        let mut state = self.state();
        for connected in state.connected.values_mut() {
            connected.show(&kinds);
        }
        state.shown = kinds;
        drop(state);
        self.repaint.request();
    }

    fn read_manifest(&self, registry: &Registry, request: ManifestRequest) {
        let mut state = self.state();
        if state.phase == Phase::Stopped {
            return;
        }
        let reader = state
            .manifests
            .get_or_insert_with(|| ManifestReader::start(&self.tasks, registry.clone(), self.repaint.clone()));
        reader.ask(request);
        drop(state);
        self.repaint.request();
    }

    async fn retry(&self, registry: &Registry, cluster: &ClusterId) -> Result<(), CoreError> {
        if let Some(link) = registry.link(cluster) {
            link.net.signal().raise(Cause::RetryNow);
            self.journal.info(cluster.as_str(), "retry now: each retry delay ends, and a failed plugin runs again");
            return Ok(());
        }
        // The connect did not start. The cause is most often the kubeconfig entry, so the
        // files load again. The reconcile then connects the cluster.
        // The copy ends the lock of the state before the read of the files.
        let env = self.state().env.clone();
        if let Some(env) = env {
            self.load_kubeconfig(&env, registry).await?;
        }
        registry.disconnect(cluster);
        Ok(())
    }

    /// Makes the connections equal to the selection: each selected cluster is connected,
    /// and no other cluster is.
    async fn reconcile(&self, registry: &Registry) -> Result<(), CoreError> {
        let clusters = registry.snapshot();
        let (selected, stopped) = {
            let mut state = self.state();
            // The stop emptied the registry. With its empty list, the selection would lose
            // each cluster. The phase changes before the registry, so this read is safe.
            if state.phase == Phase::Stopped {
                return Ok(());
            }
            state.selected.retain(|id| clusters.iter().any(|cluster| &cluster.id == id));
            let selected = state.selected.clone();
            let stopped: Vec<Connected> =
                state.connected.extract_if(|id, _| !selected.contains(id)).map(|(_, running)| running).collect();
            (selected, stopped)
        };
        // The stores of a cluster that left the selection stop here, after the lock is free.
        drop(stopped);
        for cluster in clusters {
            match (selected.contains(&cluster.id), cluster.state) {
                (true, LinkState::Off) => self.connect(registry, &cluster.id).await?,
                (false, LinkState::On(_) | LinkState::Failed(_)) => {
                    registry.disconnect(&cluster.id);
                    self.journal.info(cluster.id.as_str(), "not selected: the connections close");
                }
                (true, LinkState::On(_) | LinkState::Failed(_)) | (false, LinkState::Off) => {}
            }
        }
        self.repaint.request();
        Ok(())
    }

    async fn connect(&self, registry: &Registry, id: &ClusterId) -> Result<(), CoreError> {
        let connected = {
            let (registry, id) = (registry.clone(), id.clone());
            self.off_worker(move || registry.connect(&id)).await?
        };
        // A cluster with no stores shows no object, so a start of them that fails is a
        // connect that failed: the connections close, and each view shows the reason.
        let shown = self.state().shown.clone();
        let started = connected.and_then(|link| {
            Connected::start(self, id, &link, &shown).inspect_err(|error| registry.fail(id, error.clone()))
        });
        let running = match started {
            Ok(running) => Some(running),
            Err(error) => {
                self.journal.error(id.as_str(), format!("the connect did not start: {error}"));
                None
            }
        };
        let mut state = self.state();
        // The stores of older connections of the cluster go away in each case. A stopped
        // core keeps none: their tasks would hold the connections open.
        let old = match running.filter(|_| state.phase != Phase::Stopped) {
            Some(mut running) => {
                // The views can show other kinds and an other key view since the start of the stores.
                running.show(&state.shown);
                running.show_keys(self, state.keys.as_ref());
                state.connected.insert(id.clone(), running)
            }
            None => state.connected.remove(id),
        };
        drop(state);
        drop(old);
        Ok(())
    }
}

/// "1 context", "4 contexts": a count with its noun.
fn count(count: usize, noun: &str) -> String {
    format!("{count} {noun}{}", if count == 1 { "" } else { "s" })
}

#[cfg(test)]
mod tests {
    use fresnel_net::WallClock;
    use fresnel_testkit::Home;
    use tokio::runtime::Handle;

    use super::super::{Core, CoreParts, CoreSettings};
    use crate::cluster::ClusterId;
    use crate::journal::Journal;

    /// The stop can come while the task of the core is between two waits. The task then
    /// runs on to its next wait, and what it has in its hands must start nothing.
    #[tokio::test]
    async fn a_start_that_ends_after_the_stop_keeps_no_cluster_and_no_monitor() {
        let home = Home::new("late-start", &["kubeconfig.yaml"]).expect("a home folder");
        let text = "current-context: a\ncontexts:\n- {name: a, context: {cluster: none}}\n";
        home.write("work/kubeconfig.yaml", text).expect("the kubeconfig");
        let core = Core::new(CoreParts {
            runtime: Handle::current(),
            base_env: home.finder_env(),
            journal: Journal::new(100, WallClock::system()),
            state_file: home.path("state/state.json"),
            config_file: home.path("state/config.toml"),
            detectors: Vec::new(),
            settings: CoreSettings::default(),
        });

        core.stop();
        core.inner.prepare().await.expect("the start");
        let view = core.view();
        let read = view.journal.entries.iter().any(|entry| entry.text == "read 1 context from 1 kubeconfig file");
        assert!(read, "the start did read the file: {:?}", view.journal);
        assert_eq!(view.clusters.len(), 0, "the stop emptied the registry, and it stays empty");
        assert!(core.inner.state().monitor.is_none(), "a stopped core keeps no monitor");
    }

    /// The same for a command: the stop empties the registry, and a command that then
    /// continues must not take that for a kubeconfig with no cluster. Before, it removed
    /// each cluster from the selection and wrote an empty selection to the state file.
    #[tokio::test]
    async fn a_command_that_ends_after_the_stop_keeps_the_selection_and_writes_no_file() {
        let home = Home::new("late-command", &["kubeconfig.yaml"]).expect("a home folder");
        let text = "current-context: a\ncontexts:\n- {name: a, context: {cluster: none}}\n- {name: b, context: {}}\n";
        home.write("work/kubeconfig.yaml", text).expect("the kubeconfig");
        let core = Core::new(CoreParts {
            runtime: Handle::current(),
            base_env: home.finder_env(),
            journal: Journal::new(100, WallClock::system()),
            state_file: home.path("state/state.json"),
            config_file: home.path("state/config.toml"),
            detectors: Vec::new(),
            settings: CoreSettings::default(),
        });
        let inner = &core.inner;
        let registry = inner.prepare().await.expect("the start");
        let b = ClusterId::new("b");
        inner.select(ClusterId::new("a"), false);
        inner.select(b.clone(), true);

        core.stop();
        inner.reconcile(&registry).await.expect("the reconcile");
        inner.save_state(&registry).await.expect("no write");
        assert_eq!(inner.state().selected.iter().collect::<Vec<_>>(), [&b], "the selection of the user");
        assert!(!home.path("state/state.json").exists(), "a stopped core wrote the state file");
    }
}
