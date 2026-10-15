//! The app core of the soak, as the program runs it, in folders of the report.
//!
//! The core starts as from Finder: with a home folder of the soak whose profile gives the
//! kubeconfig of the options and the `PATH` of this process, so the login shell of the core
//! never reads the profile of the user. The state file selects the one context.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use fresnel_core::app::{Core, CoreParts, CoreSettings};
use fresnel_core::cluster::ClusterId;
use fresnel_core::journal::Journal;
use fresnel_core::net::WallClock;
use fresnel_core::net::detect::system_detectors;
use fresnel_core::net::env::ProcessEnv;
use fresnel_core::state_file::{self, SavedState};
use tokio::runtime::Handle;
use tokio::sync::Notify;

use super::args::Options;

/// The entries of the journal in memory: the count of the app.
const JOURNAL_LIMIT: usize = 10_000;
/// The `PATH` of a program that starts from Finder.
const FINDER_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

/// The core with what the soak reads of it.
pub(super) struct App {
    pub(super) core: Arc<Core>,
    pub(super) id: ClusterId,
    /// Woken at each repaint that the core asks for: a store, an index or a log changed.
    pub(super) changed: Arc<Notify>,
}

/// Writes the home folder and the state file, and starts the core.
pub(super) fn start(options: &Options, runtime: Handle) -> Result<App, String> {
    let home = options.report.join("home");
    let data = options.report.join("app");
    for folder in [&home, &data] {
        std::fs::create_dir_all(folder).map_err(|error| format!("cannot create {}: {error}", folder.display()))?;
    }
    write_profile(&home, &options.kubeconfig)?;
    let id = ClusterId::new(options.context.as_str());
    let state = SavedState { selected: BTreeSet::from([id.clone()]), tables: BTreeMap::new() };
    let state_file = data.join(state_file::FILE_NAME);
    state_file::save(&state_file, &state).map_err(|error| format!("cannot write the state file: {error}"))?;

    let mut base_env = ProcessEnv::default();
    base_env.set("HOME", home.as_os_str());
    base_env.set("PATH", FINDER_PATH);
    let core = Core::new(CoreParts {
        runtime,
        base_env,
        journal: Journal::new(JOURNAL_LIMIT, WallClock::system()),
        state_file,
        config_file: data.join("config.toml"),
        downloads: data.join("downloads"),
        detectors: system_detectors(),
        settings: CoreSettings::default(),
    });
    let changed = Arc::new(Notify::new());
    let wake = Arc::clone(&changed);
    core.start(move || wake.notify_one());
    Ok(App { core: Arc::new(core), id, changed })
}

/// The profile of each shell that the core can run: the kubeconfig files of the options and
/// the `PATH` of this process, where a credential plugin of the user is.
fn write_profile(home: &Path, kubeconfig: &[PathBuf]) -> Result<(), String> {
    let absolute = |file: &PathBuf| std::path::absolute(file).map_err(|error| format!("{}: {error}", file.display()));
    let files: Vec<PathBuf> = kubeconfig.iter().map(absolute).collect::<Result<_, _>>()?;
    let files = std::env::join_paths(files).map_err(|error| format!("a kubeconfig path: {error}"))?;
    let path = std::env::var_os("PATH").unwrap_or_default();
    let quoted = |value: &std::ffi::OsStr| format!("'{}'", value.to_string_lossy().replace('\'', r"'\''"));
    let profile = format!("export PATH={}\nexport KUBECONFIG={}\n", quoted(&path), quoted(&files));
    let write = |file: &str, text: &str| {
        std::fs::write(home.join(file), text).map_err(|error| format!("cannot write {file}: {error}"))
    };
    for file in [".zprofile", ".bash_profile", ".profile"] {
        write(file, &profile)?;
    }
    // With no file for interactive shells, zsh asks a new user some questions.
    write(".zshrc", "")
}
