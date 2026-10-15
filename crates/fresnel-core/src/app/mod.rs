//! The app core: what the window shows and what it can ask for.
//!
//! [`Core`] owns the clusters, their tasks and the journal. The UI reads a copy of the
//! state with [`Core::view`] and sends a [`Command`]. A command runs in a task of the core,
//! so the UI thread starts no network work and reads no file.
//!
//! - `run`: the task that starts the core and runs each command.
//! - `saved`: the selection of clusters and the table layouts in the state file. `config`:
//!   the settings file. `files`: the look for changes of the files.
//! - `command`: what the UI asks for.
//! - `data`: the stores and the indexes of each connected cluster. `objects`: the full
//!   objects of the kinds that the views show. `manifest`: the read of one manifest.
//! - `keys`: the object of the key view and its saves. `create`: the create of a Secret.
//! - `logs`: the log views, one session for each in the cluster of its object.
//! - `jobs`: the jobs of the operations. `access`: the permissions of the user.
//! - `watch`: one task for each connected cluster, for the journal and the repaint.
//! - `repaint`: the notifier. `health`: the one state of a cluster. `view`: the copies.
//! - `parts`: what the program gives to the core, and the settings of the core.
//! - `notices`: the notices of the system for a failed operation and a lost cluster.

mod access;
mod command;
mod config;
mod create;
mod data;
mod files;
mod health;
mod jobs;
mod keys;
mod logs;
mod manifest;
mod notices;
mod objects;
mod parts;
mod repaint;
mod run;
mod saved;
mod view;
mod watch;

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use fresnel_net::detect::{Detector, Monitor};
use fresnel_net::env::ProcessEnv;
use fresnel_net::task::{Backoff, Restart, TaskHandle, TaskName, Tasks};
use tokio::runtime::Handle;
use tokio::sync::Notify;

pub use access::{AccessView, CheckState, RulesState};
pub use command::Command;
pub use create::CreateJob;
pub use data::ClusterData;
pub use health::Health;
pub use jobs::{JobView, JobsView};
pub use keys::{KeysView, ReadState, SaveJob};
pub use manifest::{ManifestRead, ManifestRequest, ManifestState};
pub use notices::{Notice, Notifier};
pub use objects::Objects;
pub use parts::{CoreParts, CoreSettings};
pub use view::{ClusterRow, CoreView, ShellNote, Startup};

use crate::cluster::{ClusterId, Registry};
use crate::config::{Config, FileState};
use crate::journal::Journal;
use crate::keys::Target;
use crate::kinds::Kind;
use crate::logs::LogRequest;
use crate::published::Disposal;
use crate::state_file::TableLayout;
use create::SecretWriter;
use data::Connected;
use manifest::ManifestReader;
use repaint::Repaint;

/// The most commands that wait for the task of the core. The UI sends one for each click.
const COMMAND_LIMIT: usize = 256;
/// The source of the journal entries of the core itself.
const APP: &str = "app";

/// The life of the core. It goes one way.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Before [`Core::start`].
    Idle,
    Running,
    /// After [`Core::stop`].
    Stopped,
}

struct State {
    phase: Phase,
    startup: Startup,
    /// The environment of the login shell. `None` before the resolve.
    env: Option<ProcessEnv>,
    shell_problem: Option<ShellNote>,
    selected: BTreeSet<ClusterId>,
    /// What the state file has of the selection.
    saved: saved::Saved,
    /// The selection in the state file, also with the clusters that no kubeconfig file has now.
    in_state_file: BTreeSet<ClusterId>,
    /// The layout of each table that the user changed. `None` before the read of the state file.
    tables: Option<Arc<BTreeMap<String, TableLayout>>>,
    /// The settings of the user: the defaults before the read of the settings file.
    config: Arc<Config>,
    /// True after the read of the settings file, also of a file that the app does not use.
    config_read: bool,
    /// The last read of the settings file and the last save of the settings view.
    config_file: Arc<FileState>,
    /// The size and the time of change of each file at its last read. `None` before it.
    stamps: Option<Vec<files::Stamp>>,
    /// The task of the look for changes of the files. It starts after the start of the core.
    file_watch: Option<TaskHandle>,
    commands: VecDeque<Command>,
    /// The stores, the indexes and the tasks of each connected cluster.
    connected: HashMap<ClusterId, Connected>,
    /// The kinds whose full objects the views show.
    shown: BTreeSet<Kind>,
    /// The task of the manifest reads. It starts with the first request.
    manifests: Option<ManifestReader>,
    /// The object of the key view, which the core follows while the view is open.
    keys: Option<Target>,
    /// The log views that the UI shows.
    logs: Vec<LogRequest>,
    /// The number of the last save of the key view.
    saves: u64,
    /// The task of the creates of Secrets. It starts with the first create.
    creates: Option<SecretWriter>,
    /// The jobs of the operations.
    jobs: jobs::Jobs,
    detectors: Option<Vec<Box<dyn Detector>>>,
    monitor: Option<Monitor>,
    /// The tasks of the core itself. Empty before the start and after the stop.
    running: Vec<TaskHandle>,
}

struct Inner {
    journal: Journal,
    state_file: PathBuf,
    config_file: PathBuf,
    downloads: PathBuf,
    tasks: Tasks,
    runtime: Handle,
    base_env: ProcessEnv,
    /// The settings of the program. The values of the settings file replace some of them.
    settings: CoreSettings,
    /// The settings of the program with the values of the settings file, from the first
    /// read of the file on. The core builds each connection, store and view with them.
    effective: OnceLock<CoreSettings>,
    repaint: Repaint,
    /// The registry exists after the environment is known: each plugin runs with it.
    registry: OnceLock<Registry>,
    /// The thread that frees the snapshots that a reader held. It starts with the first
    /// stores, and it ends after them: this field drops after the state.
    disposal: OnceLock<Disposal>,
    state: Mutex<State>,
    /// Wakes the task of the core for a new command.
    command_sent: Notify,
    /// The function that shows a notice of the system. `None` before the program gives one.
    notifier: Mutex<Option<Notifier>>,
    /// True while the window is hidden: a lost cluster then gives a notice.
    hidden: AtomicBool,
}

impl Inner {
    /// Each use holds the lock for one small change or one copy, never across a wait.
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Gives a command to the task of the core. A stopped core drops it.
    fn queue(&self, command: Command) {
        let accepted = {
            let mut state = self.state();
            if state.phase == Phase::Stopped {
                return;
            }
            let free = state.commands.len() < COMMAND_LIMIT;
            if free {
                state.commands.push_back(command);
            }
            free
        };
        if accepted {
            self.command_sent.notify_one();
        } else {
            self.journal.error(APP, format!("a command is lost: {COMMAND_LIMIT} commands wait for the core"));
        }
    }

    /// The settings of the program with the values of the settings file, after its read.
    fn settings(&self) -> &CoreSettings {
        self.effective.get().unwrap_or(&self.settings)
    }
}

/// The app core. Its tasks stop and its connections close when this value drops.
pub struct Core {
    inner: Arc<Inner>,
}

impl Core {
    /// Builds the core. It starts no task, reads no file and opens no connection before
    /// [`Core::start`], so the window can open first.
    pub fn new(parts: CoreParts) -> Self {
        let CoreParts { runtime, base_env, journal, state_file, config_file, downloads, detectors, settings } = parts;
        let repaint = Repaint::new();
        repaint.set_limit(settings.repaints_per_second);
        let tasks = Tasks::new(runtime.clone(), repaint::reporter(journal.clone(), repaint.clone()));
        let state = State {
            phase: Phase::Idle,
            startup: Startup::Waiting,
            env: None,
            shell_problem: None,
            selected: BTreeSet::new(),
            saved: saved::Saved::Current,
            in_state_file: BTreeSet::new(),
            tables: None,
            config: Arc::default(),
            config_read: false,
            config_file: Arc::new(FileState { path: config_file.clone(), ..FileState::default() }),
            stamps: None,
            file_watch: None,
            commands: VecDeque::new(),
            connected: HashMap::new(),
            shown: BTreeSet::new(),
            manifests: None,
            keys: None,
            logs: Vec::new(),
            saves: 0,
            creates: None,
            jobs: jobs::Jobs::default(),
            detectors: Some(detectors),
            monitor: None,
            running: Vec::new(),
        };
        let inner = Inner {
            journal,
            state_file,
            config_file,
            downloads,
            tasks,
            runtime,
            base_env,
            settings,
            effective: OnceLock::new(),
            repaint,
            registry: OnceLock::new(),
            disposal: OnceLock::new(),
            state: Mutex::new(state),
            command_sent: Notify::new(),
            notifier: Mutex::new(None),
            hidden: AtomicBool::new(false),
        };
        Self { inner: Arc::new(inner) }
    }

    /// Starts the work of the core: the login shell, the kubeconfig files and the
    /// connections. `wake` asks the UI for a repaint; the core calls it from a task, a
    /// limited count of times in one second. Only the first call starts something, and a
    /// stopped core stays stopped.
    pub fn start(&self, wake: impl Fn() + Send + Sync + 'static) {
        let inner = &self.inner;
        {
            let mut state = inner.state();
            if state.phase != Phase::Idle {
                return;
            }
            state.phase = Phase::Running;
        }
        inner.repaint.set_wake(Arc::new(wake));
        let always = Restart::Always(Backoff::default());
        let notifier = inner.tasks.start(TaskName::new("repaint notifier"), always, {
            let repaint = inner.repaint.clone();
            move || repaint.clone().run()
        });
        let core = inner.tasks.start(TaskName::new("app core"), always, {
            let inner = Arc::clone(inner);
            move || run::run(Arc::clone(&inner))
        });
        let notices = inner.tasks.start(TaskName::new("notices"), always, {
            let (inner, memory) = (Arc::clone(inner), Arc::default());
            move || notices::run(Arc::clone(&inner), Arc::clone(&memory))
        });
        let mut state = inner.state();
        // A stop on some other thread can come before this line. Then the tasks stop here.
        if state.phase == Phase::Running {
            state.running = vec![notifier, core, notices];
        }
    }

    /// Gives the function that shows a notice of the system: a failed operation, and a
    /// cluster that stays lost while the window is hidden. The journal has each notice too.
    pub fn set_notifier(&self, notifier: Notifier) {
        *self.inner.notifier.lock().unwrap_or_else(PoisonError::into_inner) = Some(notifier);
    }

    /// Tells the core whether the window is hidden: minimized, or not the window of the
    /// focus. The window tells each change.
    pub fn set_window_hidden(&self, hidden: bool) {
        self.inner.hidden.store(hidden, Ordering::Relaxed);
    }

    /// The function that starts each background task of the app. The window gives it to
    /// the views for their worker threads, so a worker that ends is in the journal too.
    pub fn tasks(&self) -> Tasks {
        self.inner.tasks.clone()
    }

    /// A copy of the state of the app at one moment.
    ///
    /// The core changes the selection under the lock of its state, and the registry after
    /// it. A copy that read the registry after the lock could have the selection of one
    /// moment and the connections of a later one: a cluster selected and off. Thus the copy
    /// reads the registry under the same lock.
    pub fn view(&self) -> CoreView {
        let (startup, shell_problem, selected, tables, config, (manifest, create, jobs), mut data, clusters) = {
            let state = self.inner.state();
            let data: HashMap<ClusterId, ClusterData> =
                state.connected.iter().map(|(id, connected)| (id.clone(), connected.data())).collect();
            let manifest = state.manifests.as_ref().and_then(ManifestReader::last);
            let (create, jobs) = (state.creates.as_ref().and_then(SecretWriter::last), state.jobs.view());
            let (tables, config) = (state.tables.clone(), (Arc::clone(&state.config), Arc::clone(&state.config_file)));
            let clusters = self.inner.registry.get().map(Registry::snapshot).unwrap_or_default();
            let selected = state.selected.clone();
            let reads = (manifest, create, jobs);
            (state.startup, state.shell_problem.clone(), selected, tables, config, reads, data, clusters)
        };
        let row = |cluster: crate::cluster::ClusterView| {
            let (selected, data) = (selected.contains(&cluster.id), data.remove(&cluster.id));
            ClusterRow::new(cluster, selected, data)
        };
        CoreView {
            startup,
            shell_problem,
            clusters: clusters.into_iter().map(row).collect(),
            tables,
            config: config.0,
            config_file: config.1,
            manifest,
            create,
            jobs,
            journal: self.inner.journal.view(),
        }
    }

    /// Gives a command to the task of the core. The call returns at once. A stopped core
    /// drops the command: no task reads it.
    pub fn send(&self, command: Command) {
        self.inner.queue(command);
    }

    /// The quit sequence of the core: each task stops, each connection closes, and a
    /// running plugin process ends with its task. A stopped core does not start again.
    ///
    /// The task of the core can be in a file read at this moment, and the read ends later.
    /// Thus the task reads the phase under the lock before it keeps a result.
    pub fn stop(&self) {
        let stopped = {
            let mut state = self.inner.state();
            state.phase = Phase::Stopped;
            state.commands.clear();
            let readers = (state.manifests.take(), state.creates.take(), std::mem::take(&mut state.jobs));
            let tasks = (std::mem::take(&mut state.running), readers, (state.monitor.take(), state.file_watch.take()));
            (tasks, std::mem::take(&mut state.connected))
        };
        drop(stopped);
        if let Some(registry) = self.inner.registry.get() {
            registry.set_contexts(Vec::new());
        }
    }
}

impl Drop for Core {
    /// The tasks of the core hold its state. Without this stop, they would run to the end
    /// of the runtime.
    fn drop(&mut self) {
        self.stop();
    }
}

impl std::fmt::Debug for Core {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Core").field("startup", &self.inner.state().startup).finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests;
