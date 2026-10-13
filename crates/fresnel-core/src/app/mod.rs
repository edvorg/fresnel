//! The app core: what the window shows and what it can ask for.
//!
//! [`Core`] owns the clusters, their tasks and the journal. The UI reads a copy of the
//! state with [`Core::view`] and sends a [`Command`]. A command runs in a task of the core,
//! so the UI thread starts no network work and reads no file.
//!
//! - `run`: the task that starts the core and runs each command.
//! - `watch`: one task for each connected cluster, for the journal and the repaint.
//! - `repaint`: the notifier. `health`: the one state of a cluster. `view`: the copies.

mod health;
mod repaint;
mod run;
mod view;
mod watch;

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

use fresnel_net::NetSettings;
use fresnel_net::detect::{Detector, Monitor};
use fresnel_net::env::{EnvSettings, ProcessEnv};
use fresnel_net::task::{Backoff, End, Restart, TaskHandle, TaskName, TaskReport, Tasks};
use tokio::runtime::Handle;
use tokio::sync::Notify;

pub use health::Health;
pub use view::{ClusterRow, CoreView, ShellNote, Startup};

use crate::cluster::{ClusterId, Registry};
use crate::journal::{Journal, Level};
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
    commands: VecDeque<Command>,
    /// The state watch of each connected cluster.
    watchers: HashMap<ClusterId, TaskHandle>,
    detectors: Option<Vec<Box<dyn Detector>>>,
    monitor: Option<Monitor>,
    /// The tasks of the core itself. Empty before the start and after the stop.
    running: Vec<TaskHandle>,
}

struct Inner {
    journal: Journal,
    tasks: Tasks,
    runtime: Handle,
    base_env: ProcessEnv,
    settings: CoreSettings,
    repaint: Repaint,
    /// The registry exists after the environment is known: each plugin runs with it.
    registry: OnceLock<Registry>,
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
        let CoreParts { runtime, base_env, journal, detectors, settings } = parts;
        let repaint = Repaint::new();
        let tasks = Tasks::new(runtime.clone(), reporter(journal.clone(), repaint.clone()));
        let state = State {
            phase: Phase::Idle,
            startup: Startup::Waiting,
            env: None,
            shell_problem: None,
            selected: BTreeSet::new(),
            commands: VecDeque::new(),
            watchers: HashMap::new(),
            detectors: Some(detectors),
            monitor: None,
            running: Vec::new(),
        };
        let inner = Inner {
            journal,
            tasks,
            runtime,
            base_env,
            settings,
            repaint,
            registry: OnceLock::new(),
            state: Mutex::new(state),
            command_sent: Notify::new(),
        };
        Self { inner: Arc::new(inner) }
    }

    /// Starts the work of the core: the login shell, the kubeconfig files and the
    /// connections. `wake` asks the UI for a repaint; the core calls it from a task, a
    /// limited count of times in one second. Only the first call starts something.
    pub fn start(&self, wake: impl Fn() + Send + Sync + 'static) {
        let inner = &self.inner;
        if std::mem::replace(&mut inner.state().phase, Phase::Running) != Phase::Idle {
            return;
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

    /// A copy of the state of the app.
    pub fn view(&self) -> CoreView {
        let (startup, shell_problem, selected) = {
            let state = self.inner.state();
            (state.startup, state.shell_problem.clone(), state.selected.clone())
        };
        let clusters = self.inner.registry.get().map(Registry::snapshot).unwrap_or_default();
        let row = |cluster: crate::cluster::ClusterView| {
            let selected = selected.contains(&cluster.id);
            ClusterRow::new(cluster, selected)
        };
        CoreView {
            startup,
            shell_problem,
            clusters: clusters.into_iter().map(row).collect(),
            journal: self.inner.journal.view(),
        }
    }

    /// Gives a command to the task of the core. The call returns at once.
    pub fn send(&self, command: Command) {
        let accepted = {
            let mut state = self.inner.state();
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
    pub fn stop(&self) {
        let stopped = {
            let mut state = self.inner.state();
            state.phase = Phase::Stopped;
            state.commands.clear();
            (std::mem::take(&mut state.running), std::mem::take(&mut state.watchers), state.monitor.take())
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

/// The journal entry for each run of a task that ended (design rule 1).
fn reporter(journal: Journal, repaint: Repaint) -> impl Fn(TaskReport) + Send + Sync + 'static {
    move |report: TaskReport| {
        let level = match (&report.end, report.restart_in) {
            (End::Failed(_) | End::Panicked(_), _) => Level::Error,
            // A task that runs again must not end, and a task cannot run with no runtime.
            (End::Done, Some(_)) | (End::Stopped, _) => Level::Warning,
            (End::Done, None) => Level::Info,
        };
        journal.push(level, "task", report.to_string());
        repaint.request();
    }
}

#[cfg(test)]
mod tests {
    use fresnel_net::WallClock;
    use tokio::time::sleep;

    use super::*;
    use crate::error::CoreError;

    fn core() -> Core {
        Core::new(CoreParts {
            runtime: Handle::current(),
            base_env: ProcessEnv::default(),
            journal: Journal::new(100, WallClock::system()),
            detectors: Vec::new(),
            settings: CoreSettings::default(),
        })
    }

    fn journal(core: &Core) -> Vec<(Level, String)> {
        core.view().journal.entries.iter().map(|entry| (entry.level, entry.text.clone())).collect()
    }

    /// Design rule 1: a task that ends gets a journal entry.
    #[tokio::test(start_paused = true)]
    async fn each_run_of_a_task_that_ends_is_in_the_journal_with_its_level() {
        let core = core();
        let tasks = &core.inner.tasks;
        let ends = |fails: bool| move || async move { if fails { Err(CoreError::invalid("no luck")) } else { Ok(()) } };
        let job = tasks.start(TaskName::new("job"), Restart::Never, ends(false));
        let broken = tasks.start(TaskName::new("broken job"), Restart::Never, ends(true));
        let pause = Backoff { first: Duration::from_secs(600), ..Backoff::default() };
        let service = tasks.start(TaskName::new("service"), Restart::Always(pause), ends(false));
        sleep(Duration::from_secs(1)).await;

        let mut entries = journal(&core);
        entries.sort();
        assert_eq!(entries.len(), 3, "{entries:?}");
        assert_eq!(entries[0], (Level::Info, "task job ended; no restart".to_owned()));
        assert_eq!(entries[1].0, Level::Warning, "a task that must stay alive ended: {entries:?}");
        assert!(entries[1].1.starts_with("task service ended; restart in "), "{entries:?}");
        assert_eq!(entries[2], (Level::Error, "task broken job failed: no luck; no restart".to_owned()));
        drop((job, broken, service));
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
    }
}
