//! The app core: what the window shows and what it can ask for.
//!
//! [`Core`] owns the clusters, their tasks and the journal. The UI reads a copy of the
//! state with [`Core::view`] and sends a [`Command`]. A command runs in a task of the core,
//! so the UI thread starts no network work and reads no file.
//!
//! - `run`: the task that starts the core and runs each command.
//! - `saved`: the selection of clusters and the table layouts in the state file.
//! - `data`: the stores and the indexes of each connected cluster.
//! - `watch`: one task for each connected cluster, for the journal and the repaint.
//! - `repaint`: the notifier. `health`: the one state of a cluster. `view`: the copies.

mod data;
mod health;
mod repaint;
mod run;
mod saved;
mod view;
mod watch;

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

use fresnel_net::NetSettings;
use fresnel_net::detect::{Detector, Monitor};
use fresnel_net::env::{EnvSettings, ProcessEnv};
use fresnel_net::task::{Backoff, Reporter, Restart, TaskHandle, TaskName, TaskReport, Tasks};
use tokio::runtime::Handle;
use tokio::sync::Notify;

pub use data::ClusterData;
pub use health::Health;
pub use view::{ClusterRow, CoreView, ShellNote, Startup};

use crate::cluster::{ClusterId, Registry};
use crate::journal::Journal;
use crate::published::Disposal;
use crate::state_file::TableLayout;
use crate::store::StoreSettings;
use data::Connected;
use repaint::Repaint;

/// The most commands that wait for the task of the core. The UI sends one for each click.
const COMMAND_LIMIT: usize = 256;
/// The source of the journal entries of the core itself.
const APP: &str = "app";

/// The settings of the core.
#[derive(Clone, Debug)]
pub struct CoreSettings {
    /// The read of the login shell environment.
    pub env: EnvSettings,
    /// The connection layer of each cluster.
    pub net: NetSettings,
    /// The stores of each cluster.
    pub stores: StoreSettings,
    /// The wake detector and the network detector look each time this period ends.
    pub detect_period: Duration,
    /// The most repaints in one second that the core asks the UI for.
    pub repaints_per_second: u32,
}

impl Default for CoreSettings {
    fn default() -> Self {
        Self {
            env: EnvSettings::default(),
            net: NetSettings::default(),
            stores: StoreSettings::default(),
            detect_period: Duration::from_secs(2),
            repaints_per_second: 60,
        }
    }
}

/// What the program gives to the core.
pub struct CoreParts {
    /// The runtime of each task.
    pub runtime: Handle,
    /// The environment of the process. The login shell starts from it.
    pub base_env: ProcessEnv,
    /// The journal. The program gives it the log file as its sink.
    pub journal: Journal,
    /// The state file: the core reads the selection and the table layouts of the last run
    /// from it, and writes each change of them to it.
    pub state_file: PathBuf,
    /// The detectors of a wake and of a network change.
    pub detectors: Vec<Box<dyn Detector>>,
    /// The settings.
    pub settings: CoreSettings,
}

/// What the UI asks the core to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// Selects a cluster or removes it from the selection. The core connects each
    /// selected cluster.
    Select {
        /// The cluster.
        cluster: ClusterId,
        /// True to select it.
        selected: bool,
    },
    /// "Retry now" for one cluster: each retry delay ends, and a failed plugin runs again.
    /// For a cluster whose connect did not start, the kubeconfig files load again.
    RetryNow(ClusterId),
    /// Saves the layout of a table that the user changed. An empty layout removes the table
    /// from the state file.
    SaveTable {
        /// The name of the table.
        table: String,
        /// What the user changed of it.
        layout: TableLayout,
    },
}

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
    commands: VecDeque<Command>,
    /// The stores, the indexes and the tasks of each connected cluster.
    connected: HashMap<ClusterId, Connected>,
    detectors: Option<Vec<Box<dyn Detector>>>,
    monitor: Option<Monitor>,
    /// The tasks of the core itself. Empty before the start and after the stop.
    running: Vec<TaskHandle>,
}

struct Inner {
    journal: Journal,
    state_file: PathBuf,
    tasks: Tasks,
    runtime: Handle,
    base_env: ProcessEnv,
    settings: CoreSettings,
    repaint: Repaint,
    /// The registry exists after the environment is known: each plugin runs with it.
    registry: OnceLock<Registry>,
    /// The thread that frees the snapshots that a reader held. It starts with the first
    /// stores, and it ends after them: this field drops after the state.
    disposal: OnceLock<Disposal>,
    state: Mutex<State>,
    /// Wakes the task of the core for a new command.
    command_sent: Notify,
}

impl Inner {
    /// Each use holds the lock for one small change or one copy, never across a wait.
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
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
        let CoreParts { runtime, base_env, journal, state_file, detectors, settings } = parts;
        let repaint = Repaint::new();
        let tasks = Tasks::new(runtime.clone(), reporter(journal.clone(), repaint.clone()));
        let state = State {
            phase: Phase::Idle,
            startup: Startup::Waiting,
            env: None,
            shell_problem: None,
            selected: BTreeSet::new(),
            saved: saved::Saved::Current,
            in_state_file: BTreeSet::new(),
            tables: None,
            commands: VecDeque::new(),
            connected: HashMap::new(),
            detectors: Some(detectors),
            monitor: None,
            running: Vec::new(),
        };
        let inner = Inner {
            journal,
            state_file,
            tasks,
            runtime,
            base_env,
            settings,
            repaint,
            registry: OnceLock::new(),
            disposal: OnceLock::new(),
            state: Mutex::new(state),
            command_sent: Notify::new(),
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
        let period = Duration::from_secs(1) / inner.settings.repaints_per_second.max(1);
        let always = Restart::Always(Backoff::default());
        let notifier = inner.tasks.start(TaskName::new("repaint notifier"), always, {
            let repaint = inner.repaint.clone();
            move || repaint.clone().run(period)
        });
        let core = inner.tasks.start(TaskName::new("app core"), always, {
            let inner = Arc::clone(inner);
            move || run::run(Arc::clone(&inner))
        });
        let mut state = inner.state();
        // A stop on some other thread can come before this line. Then the tasks stop here.
        if state.phase == Phase::Running {
            state.running = vec![notifier, core];
        }
    }

    /// The function that starts each background task of the app. The window gives it to
    /// the views for their worker threads, so a worker that ends is in the journal too.
    pub fn tasks(&self) -> Tasks {
        self.inner.tasks.clone()
    }

    /// A copy of the state of the app.
    pub fn view(&self) -> CoreView {
        let (startup, shell_problem, selected, tables, mut data) = {
            let state = self.inner.state();
            let data: HashMap<ClusterId, ClusterData> =
                state.connected.iter().map(|(id, connected)| (id.clone(), connected.data())).collect();
            (state.startup, state.shell_problem.clone(), state.selected.clone(), state.tables.clone(), data)
        };
        let clusters = self.inner.registry.get().map(Registry::snapshot).unwrap_or_default();
        let row = |cluster: crate::cluster::ClusterView| {
            let selected = selected.contains(&cluster.id);
            let data = data.remove(&cluster.id);
            ClusterRow::new(cluster, selected, data)
        };
        CoreView {
            startup,
            shell_problem,
            clusters: clusters.into_iter().map(row).collect(),
            tables,
            journal: self.inner.journal.view(),
        }
    }

    /// Gives a command to the task of the core. The call returns at once. A stopped core
    /// drops the command: no task reads it.
    pub fn send(&self, command: Command) {
        let accepted = {
            let mut state = self.inner.state();
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
            self.inner.command_sent.notify_one();
        } else {
            self.inner.journal.error(APP, format!("a command is lost: {COMMAND_LIMIT} commands wait for the core"));
        }
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
            (std::mem::take(&mut state.running), std::mem::take(&mut state.connected), state.monitor.take())
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

/// The journal takes the report of each task run that ended (design rule 1), and the UI
/// gets a repaint for the new entry.
fn reporter(journal: Journal, repaint: Repaint) -> impl Reporter {
    move |report: TaskReport| {
        journal.report(report);
        repaint.request();
    }
}

#[cfg(test)]
mod tests {
    use fresnel_net::WallClock;
    use tokio::time::sleep;

    use super::*;
    use crate::error::CoreError;
    use crate::journal::Level;

    fn core() -> Core {
        Core::new(CoreParts {
            runtime: Handle::current(),
            base_env: ProcessEnv::default(),
            journal: Journal::new(100, WallClock::system()),
            // No test of this module starts the core, so nothing reads or writes the file.
            state_file: PathBuf::from("/no/such/folder/state.json"),
            detectors: Vec::new(),
            settings: CoreSettings::default(),
        })
    }

    fn journal(core: &Core) -> Vec<(Level, String)> {
        core.view().journal.entries.iter().map(|entry| (entry.level, entry.text.clone())).collect()
    }

    /// Design rule 1: the journal of the core is the receiver of the reports of its tasks.
    #[tokio::test(start_paused = true)]
    async fn a_task_of_the_core_that_ends_is_in_the_journal() {
        let core = core();
        let fails = || async { Err(CoreError::invalid("no luck")) };
        let broken = core.inner.tasks.start(TaskName::new("broken job"), Restart::Never, fails);
        sleep(Duration::from_secs(1)).await;
        assert_eq!(journal(&core), [(Level::Error, "task broken job failed: no luck; no restart".to_owned())]);
        drop(broken);
    }

    #[tokio::test]
    async fn the_commands_that_wait_have_a_limit_and_a_lost_command_is_in_the_journal() {
        let core = core();
        let command = || Command::RetryNow(ClusterId::new("ams"));
        (0..COMMAND_LIMIT).for_each(|_| core.send(command()));
        assert_eq!(journal(&core), []);
        core.send(command());
        assert_eq!(journal(&core), [(Level::Error, "a command is lost: 256 commands wait for the core".to_owned())]);
        assert_eq!(core.inner.state().commands.len(), COMMAND_LIMIT);

        // A stopped core takes no command: no task reads it.
        core.stop();
        core.send(command());
        let waiting = core.inner.state().commands.len();
        assert_eq!((waiting, journal(&core).len()), (0, 1));
    }
}
