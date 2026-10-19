//! The look for changes of the kubeconfig files and the settings file (plan section 12.5).
//!
//! macOS has a file system event API, but an editor that saves a new file under the old
//! name, a symbolic link and a folder that appears later each need their own case there. A
//! look at the size and the time of change of each file in each period has none of these
//! cases, and its cost is a few calls for each period. A change sends one
//! `Command::ReloadFiles`, so the reload runs in the task of the core between two commands.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use fresnel_net::env::ProcessEnv;
use fresnel_net::task::{Backoff, Restart, TaskHandle, TaskName};

use super::Inner;
use super::command::Command;
use crate::config::apply::{expand, watch_period};
use crate::config::table::Setting;
use crate::error::CoreError;
use crate::kubeconfig;

/// The size and the time of change of one file. `None` for a file that is absent.
pub(super) type Stamp = (PathBuf, Option<(u64, SystemTime)>);

/// Where the kubeconfig files of the user are: the environment of the login shell and the
/// files of the settings. [`Sources::paths`] looks at the disk, so it runs on a worker thread.
pub(super) struct Sources {
    env: ProcessEnv,
    extra: Vec<PathBuf>,
}

impl Sources {
    /// The kubeconfig files: `KUBECONFIG` of the login shell, `.kube/config` of the home
    /// folder of kubectl and then the files of the settings.
    pub(super) fn paths(&self) -> Vec<PathBuf> {
        let mut sources = kubeconfig::default_sources(&self.env, Path::is_file);
        for path in &self.extra {
            if !sources.contains(path) {
                sources.push(path.clone());
            }
        }
        sources
    }
}

impl Inner {
    /// The places of the kubeconfig files for the environment `env`.
    pub(super) fn kubeconfig_sources(&self, env: &ProcessEnv) -> Sources {
        let home = env.home().map(PathBuf::from);
        let extra = self.state().config.list(Setting::KubeconfigFiles);
        let extra = extra.iter().map(|path| expand(path, home.as_deref())).collect();
        Sources { env: env.clone(), extra }
    }

    /// The files that the look follows: the settings file and the kubeconfig files. None
    /// before the login shell gave its environment.
    fn watched(&self) -> (PathBuf, Option<Sources>) {
        let env = self.state().env.clone();
        (self.config_file.clone(), env.map(|env| self.kubeconfig_sources(&env)))
    }

    /// Takes the stamps of the files before a read of them, as the point from which the look
    /// sees a change. A save of the settings view thus gives no second read, and a change
    /// during the read gives one more.
    pub(super) async fn mark_files(&self) -> Result<(), CoreError> {
        let watched = self.watched();
        let stamps = self.off_worker(move || stamps(watched)).await?;
        self.state().stamps = Some(stamps);
        Ok(())
    }
}

/// Starts the task of the look.
pub(super) fn start(inner: &Arc<Inner>) -> TaskHandle {
    let always = Restart::Always(Backoff::default());
    inner.tasks.start(TaskName::new("file watch"), always, {
        let inner = Arc::clone(inner);
        move || look(Arc::clone(&inner))
    })
}

async fn look(inner: Arc<Inner>) -> Result<(), CoreError> {
    loop {
        let period = watch_period(&inner.state().config);
        tokio::time::sleep(period).await;
        let watched = inner.watched();
        let now = inner.off_worker(move || stamps(watched)).await?;
        let changed = {
            let mut state = inner.state();
            let changed = state.stamps.as_ref().is_some_and(|before| *before != now);
            state.stamps = Some(now);
            changed
        };
        if changed {
            inner.queue(Command::ReloadFiles);
        }
    }
}

/// The stamp of the settings file and of each kubeconfig file. A file that the look cannot
/// read has no stamp, as an absent one. With no environment yet, the look follows nothing.
fn stamps((config, sources): (PathBuf, Option<Sources>)) -> Vec<Stamp> {
    let Some(sources) = sources else {
        return Vec::new();
    };
    let stamp = |path: &PathBuf| {
        let metadata = std::fs::metadata(path).ok()?;
        Some((metadata.len(), metadata.modified().ok()?))
    };
    let paths = std::iter::once(config).chain(sources.paths());
    paths.map(|path| (path.clone(), stamp(&path))).collect()
}
