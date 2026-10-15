//! The settings file in the core (plan section 12.5): the read at the start, a read again
//! after a change of the file, and the saves of the settings view.
//!
//! The core reads `config.toml` before the login shell, so the keys of the user act from
//! the first frames. Each problem of the file goes to the journal with its line and column.
//! The views read the settings in `CoreView::config` and act on a change at once. The core
//! builds its connections, stores and views with the settings of the first read: a change of
//! a group of `Group::at_start` acts at the next start, and the journal says so.

use std::path::PathBuf;
use std::sync::Arc;

use super::{APP, Inner};
use crate::cluster::Registry;
use crate::config::apply::apply;
use crate::config::table::Setting;
use crate::config::{self, Config, Edit, FileRead, Loaded, SaveState};
use crate::error::CoreError;

impl Inner {
    /// Reads the settings file and makes the settings of the core with it, if no run of the
    /// task did this before.
    pub(super) async fn load_config(&self) -> Result<(), CoreError> {
        if self.state().config_read {
            return Ok(());
        }
        let config = self.read_config().await?;
        self.effective.get_or_init(|| apply(&config, &self.settings, self.home().as_deref()));
        self.state().config_read = true;
        Ok(())
    }

    /// Reads the settings file again after a change of it. The settings of the views act
    /// at once.
    pub(super) async fn reload_config(&self) -> Result<(), CoreError> {
        let config = self.read_config().await?;
        let wanted = apply(&config, &self.settings, self.home().as_deref());
        // The settings of the core have no equality of their own, and their text has each field.
        if format!("{wanted:?}") != format!("{:?}", self.settings()) {
            let text = "the network, credential plugin, metrics, log and Events settings act at the next start";
            self.journal.info(APP, format!("{}: {text}", self.config_file.display()));
        }
        Ok(())
    }

    /// Writes the changes of the settings view into the file. The caller reads it again.
    pub(super) async fn edit_config(&self, edits: Vec<Edit>) {
        let number = {
            let mut state = self.state();
            let number = state.config_file.save.as_ref().map_or(1, |save| save.number + 1);
            Arc::make_mut(&mut state.config_file).save = Some(SaveState { number, result: None });
            number
        };
        self.repaint.request();
        let path = self.config_file.clone();
        // A write that did not end is a result too: the view waits for one.
        let written = match self.off_worker(move || config::edit::write(&path, &edits)).await {
            Ok(written) => written,
            Err(error) => Err(error.to_string()),
        };
        let file = self.config_file.display();
        match &written {
            Ok(()) => self.journal.info(APP, format!("{file}: saved by the settings view")),
            Err(error) => self.journal.warning(APP, format!("{file}: the settings view did not save: {error}")),
        }
        Arc::make_mut(&mut self.state().config_file).save = Some(SaveState { number, result: Some(written) });
        self.repaint.request();
    }

    /// Reads the settings file and the kubeconfig files again, after a change of one of
    /// them. The caller then connects the clusters of the selection.
    pub(super) async fn reload_files(&self, registry: &Registry) -> Result<(), CoreError> {
        // The stamps come before the reads: a change during the reads then gives one more read.
        self.mark_files().await?;
        self.reload_config().await?;
        let env = self.state().env.clone();
        if let Some(env) = env {
            self.load_kubeconfig(&env, registry).await?;
        }
        Ok(())
    }

    /// Reads the file, gives its settings to the views and each problem to the journal.
    /// Gives the settings in use: those of the file, or the defaults for a file that the app
    /// does not use.
    async fn read_config(&self) -> Result<Config, CoreError> {
        let path = self.config_file.clone();
        let loaded = self.off_worker(move || config::load(&path)).await?;
        let file = self.config_file.display();
        let read = match loaded {
            Ok(None) => {
                self.journal.info(APP, format!("{file}: no file; each setting has its default"));
                self.state().config = Arc::default();
                FileRead::Absent
            }
            Ok(Some(loaded)) => self.report(loaded),
            Err(error) => {
                self.journal.warning(APP, format!("{file}: {error}; {}", self.kept()));
                FileRead::Failed(error.to_string())
            }
        };
        let config = {
            let mut state = self.state();
            Arc::make_mut(&mut state.config_file).read = read;
            Arc::clone(&state.config)
        };
        // The limits act at once. A file with no limit gives the limit of the program again.
        let limit = config.set(Setting::JournalEntries).map(|_| config.number(Setting::JournalEntries));
        self.journal.set_limit(limit.map(|limit| usize::try_from(limit).unwrap_or(usize::MAX)));
        let repaints = config.set(Setting::RepaintLimit).map(|_| config.number(Setting::RepaintLimit));
        let repaints = repaints.map_or(self.settings.repaints_per_second, |n| u32::try_from(n).unwrap_or(u32::MAX));
        self.repaint.set_limit(repaints);
        self.repaint.request();
        Ok(Config::clone(&config))
    }

    /// Gives the settings of a file to the views, and each problem of it to the journal. A
    /// file that the app does not use leaves the settings as they are (design rule 5): a
    /// mistake in an editor must not make a read-only cluster writable.
    fn report(&self, loaded: Loaded) -> FileRead {
        let file = self.config_file.display();
        let Loaded { config, problems, used } = loaded;
        for problem in &problems {
            if used {
                self.journal.warning(APP, format!("{file}: {problem}"));
            } else {
                self.journal.error(APP, format!("{file}: {problem}; {}", self.kept()));
            }
        }
        if used && problems.is_empty() {
            self.journal.info(APP, format!("{file}: read"));
        }
        if used {
            self.state().config = Arc::new(config);
        }
        FileRead::Read { problems, used }
    }

    /// What stays after a file that the app does not use: the defaults at the start, the
    /// settings of the last read after it.
    fn kept(&self) -> &'static str {
        if self.state().config_read {
            "the app keeps the settings of its last read"
        } else {
            "the app uses no setting of the file"
        }
    }

    /// The home folder of the user, for a path of the settings that starts with `~/`.
    pub(super) fn home(&self) -> Option<PathBuf> {
        self.base_env.get("HOME").filter(|home| !home.is_empty()).map(PathBuf::from)
    }
}

#[cfg(test)]
mod tests {
    use fresnel_net::WallClock;
    use fresnel_testkit::Home;
    use tokio::runtime::Handle;

    use super::super::{Core, CoreParts, CoreSettings};
    use crate::journal::Journal;

    /// The limits of the journal and of the repaints in the file act at once, and a file with
    /// no limit gives the limit of the program again.
    #[tokio::test]
    async fn the_limits_of_the_file_act_at_once_and_after_them_the_limits_of_the_program() {
        let home = Home::new("limits", &[]).expect("a home folder");
        let text = "version = 1\n[diagnostics]\njournal_entries = 1000\n[appearance]\nrepaints_per_second = 10\n";
        home.write("state/config.toml", text).expect("the file");
        let journal = Journal::new(2000, WallClock::system());
        let core = Core::new(CoreParts {
            runtime: Handle::current(),
            base_env: home.finder_env(),
            journal: journal.clone(),
            state_file: home.path("state/state.json"),
            config_file: home.path("state/config.toml"),
            downloads: home.path("Downloads"),
            detectors: Vec::new(),
            settings: CoreSettings::default(),
        });
        let fill = || (0..3000).for_each(|number| journal.info("test", number.to_string()));
        core.inner.load_config().await.expect("the read");
        fill();
        assert_eq!((journal.view().entries.len(), core.inner.repaint.limit()), (1000, 10), "the limits of the file");
        home.write("state/config.toml", "version = 1\n").expect("a file with no limit");
        core.inner.reload_config().await.expect("the read");
        fill();
        assert_eq!((journal.view().entries.len(), core.inner.repaint.limit()), (2000, 60), "of the program");
    }
}
