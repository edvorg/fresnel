//! The start of the program before its window, and its quit.
//!
//! [`Boot::start`] takes the lock of the process, opens the log file, installs the panic
//! report and builds the runtime and the app core. It starts no network work: the core
//! starts when the window is open. [`Boot::quit`] is the quit sequence.

use std::fmt::{self, Display};
use std::io;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use fresnel_core::app::{Core, CoreParts, CoreSettings};
use fresnel_core::journal::Journal;
use fresnel_core::net::WallClock;
use fresnel_core::net::detect::system_detectors;
use fresnel_core::net::env::ProcessEnv;
use fresnel_core::{config, state_file};
use tokio::runtime::{Builder, Runtime};

use crate::folders::{Build, Folders};
use crate::lock::{LockError, ProcessLock};
use crate::log_file::{LogFile, Rotation};
use crate::panic_report::{self, Reports};
use crate::{VERSION, platform};

/// The source of the journal entries of the program.
const APP: &str = "app";
/// The count of entries that the journal keeps in memory. The log file keeps all.
const JOURNAL_LIMIT: usize = 10_000;
/// The threads of the runtime. The network work of some clusters is light.
const WORKER_THREADS: usize = 4;
/// The longest wait for the tasks of the runtime at the quit. A task ends at once; only an
/// address lookup of the system and a read of a file can block, and they cannot be stopped.
const EXIT_LIMIT: Duration = Duration::from_secs(1);

/// Why the program did not start.
#[derive(Debug)]
pub enum BootError {
    /// The environment has no variable that names the folders of the app.
    NoHome,
    /// A folder of the app could not be made.
    Folders(Folders, io::Error),
    /// The lock of the process is not free.
    Lock(LockError),
    /// The log file could not be opened.
    LogFile(PathBuf, io::Error),
    /// The runtime did not start.
    Runtime(io::Error),
}

impl Display for BootError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoHome => write!(f, "the environment has no {}", platform::FOLDER_VARIABLES),
            Self::Folders(folders, error) => {
                write!(f, "cannot make {} and {}: {error}", folders.data.display(), folders.logs.display())
            }
            Self::Lock(error) => Display::fmt(error, f),
            Self::LogFile(folder, error) => write!(f, "cannot open the log file in {}: {error}", folder.display()),
            Self::Runtime(error) => write!(f, "cannot start the runtime: {error}"),
        }
    }
}

/// The program with no window: its folders, its journal, its runtime and the app core.
#[derive(Debug)]
pub struct Boot {
    folders: Folders,
    journal: Journal,
    core: Core,
    /// `None` after the quit.
    runtime: Mutex<Option<Runtime>>,
    _lock: ProcessLock,
}

impl Boot {
    /// Starts the program for the environment of the process. A program from Finder has
    /// only `HOME` and a short `PATH` there; the core reads the rest from the login shell.
    pub fn start(base_env: ProcessEnv) -> Result<Self, BootError> {
        let folders = Folders::of(&base_env, Build::current()).ok_or(BootError::NoHome)?;
        folders.create().map_err(|error| BootError::Folders(folders.clone(), error))?;
        let lock = ProcessLock::take(&folders.data).map_err(BootError::Lock)?;
        let log = LogFile::open(&folders.logs, Rotation::default());
        let log = log.map_err(|error| BootError::LogFile(folders.logs.clone(), error))?;
        let journal = Journal::with_sink(JOURNAL_LIMIT, WallClock::system(), log);
        let earlier_reports = panic_report::prune(&folders.logs);
        Reports::new(&folders.logs, VERSION).install();

        let build = if Build::current() == Build::Debug { " (debug build)" } else { "" };
        let (data, logs) = (folders.data.display(), folders.logs.display());
        journal.info(APP, format!("fresnel {VERSION}{build} started; settings in {data}, logs in {logs}"));
        if earlier_reports > 0 {
            journal.warning(APP, format!("{earlier_reports} panic reports of earlier runs are in {logs}"));
        }

        let mut runtime = Builder::new_multi_thread();
        runtime.worker_threads(WORKER_THREADS).thread_name("fresnel-task").enable_all();
        let runtime = runtime.build().map_err(BootError::Runtime)?;
        let core = Core::new(CoreParts {
            runtime: runtime.handle().clone(),
            base_env,
            journal: journal.clone(),
            state_file: folders.data.join(state_file::FILE_NAME),
            config_file: folders.data.join(config::FILE_NAME),
            downloads: folders.downloads.clone(),
            detectors: system_detectors(),
            settings: CoreSettings::default(),
        });
        Ok(Self { folders, journal, core, runtime: Mutex::new(Some(runtime)), _lock: lock })
    }

    /// The app core. The window starts it and reads it.
    pub fn core(&self) -> &Core {
        &self.core
    }

    /// The journal, for an entry of the program itself.
    pub fn journal(&self) -> &Journal {
        &self.journal
    }

    /// The folders of the app.
    pub fn folders(&self) -> &Folders {
        &self.folders
    }

    /// The quit sequence (plan section 11.6): each terminal ends its program in the pod,
    /// with a limit; the core stops its tasks, closes each connection and ends a plugin that
    /// runs; the runtime ends; the log file reaches the disk. Only the first call does
    /// something. The lock of the process stays to the end of the process.
    pub fn quit(&self) {
        let Some(runtime) = self.runtime.lock().unwrap_or_else(PoisonError::into_inner).take() else {
            return;
        };
        // A stop of the task of a terminal would leave its shell in the pod, and the
        // privileged pod of a node shell on its node.
        if !runtime.block_on(self.core.close_terminals()) {
            self.journal.warning(APP, "quit: a terminal did not end its program in the pod in time");
        }
        self.journal.info(APP, "quit: the tasks stop and the connections close");
        self.core.stop();
        runtime.shutdown_timeout(EXIT_LIMIT);
        self.journal.info(APP, "quit complete");
        self.journal.flush();
    }
}
