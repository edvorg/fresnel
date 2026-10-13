//! The task of the core: its start, and then each command of the UI.
//!
//! The start reads the login shell environment and the kubeconfig files. A command can read
//! the certificate files of a cluster. A read of a file can wait for a slow disk, so it
//! runs on the threads for blocking calls. A worker thread of the runtime that waits can
//! stop the timers and the sockets of each cluster.

use std::sync::Arc;

use fresnel_net::Cause;
use fresnel_net::detect::{Disturbance, Monitor};
use fresnel_net::env::{self, EnvSource, ProcessEnv};

use super::view::{ShellNote, Startup};
use super::{APP, Command, Inner, Phase, watch};
use crate::cluster::{ClusterId, LinkState, Registry};
use crate::error::CoreError;
use crate::kubeconfig;

/// One run of the task. After an error or a panic, the next run continues from the state
/// that the core has: a start that was complete does not run again.
pub(super) async fn run(inner: Arc<Inner>) -> Result<(), CoreError> {
    let registry = inner.prepare().await?;
    loop {
        // A command that arrives after this read leaves a permit, so the wait ends at once.
        while let Some(command) = inner.next_command() {
            inner.apply(&registry, command).await?;
        }
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
        let env = self.environment().await;
        let registry = self.registry.get_or_init(|| {
            Registry::new(self.tasks.clone(), self.runtime.clone(), env.clone(), self.settings.net.clone())
        });
        self.enter(Startup::Kubeconfig);
        let current = self.load_kubeconfig(&env, registry).await?;
        // The cluster of `kubectl` is the selection at the start. The user selects the others.
        self.state().selected.extend(current.map(ClusterId::new));
        self.start_monitor(registry);
        self.reconcile(registry).await?;
        self.enter(Startup::Ready);
        Ok(registry.clone())
    }

    /// Runs a call that reads files on a thread for blocking calls, and waits for its result.
    async fn off_worker<T: Send + 'static>(&self, read: impl FnOnce() -> T + Send + 'static) -> Result<T, CoreError> {
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
        let state = self.state();
        // The stop emptied the registry. A load that ends after the stop must not fill it again.
        if state.phase != Phase::Stopped {
            registry.set_contexts(loaded.contexts);
        }
        Ok(current)
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
            Command::Select { cluster, selected: true } => {
                self.state().selected.insert(cluster);
            }
            Command::Select { cluster, selected: false } => {
                self.state().selected.remove(&cluster);
            }
            Command::RetryNow(cluster) => self.retry(registry, &cluster).await?,
        }
        self.reconcile(registry).await
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
        let selected = {
            let mut state = self.state();
            state.selected.retain(|id| clusters.iter().any(|cluster| &cluster.id == id));
            let selected = state.selected.clone();
            state.watchers.retain(|id, _| selected.contains(id));
            selected
        };
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
        let watcher = match connected {
            Ok(link) => Some(watch::start(&self.tasks, id, link.net, self.journal.clone(), self.repaint.clone())),
            Err(error) => {
                self.journal.error(id.as_str(), format!("the connect did not start: {error}"));
                None
            }
        };
        let mut state = self.state();
        // A watch of older connections of the cluster goes away in each case. A stopped
        // core keeps no watch: the watch would hold the connections open.
        let old = match watcher.filter(|_| state.phase != Phase::Stopped) {
            Some(watcher) => state.watchers.insert(id.clone(), watcher),
            None => state.watchers.remove(id),
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
}
