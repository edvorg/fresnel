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
    pub(super) async fn edit_config(&self, edits: Vec<Edit>) -> Result<(), CoreError> {
        let number = {
            let mut state = self.state();
            let number = state.config_file.save.as_ref().map_or(1, |save| save.number + 1);
            Arc::make_mut(&mut state.config_file).save = Some(SaveState { number, result: None });
            number
        };
        self.repaint.request();
        let path = self.config_file.clone();
        let written = self.off_worker(move || config::edit::write(&path, &edits)).await?;
        let file = self.config_file.display();
        match &written {
            Ok(()) => self.journal.info(APP, format!("{file}: saved by the settings view")),
            Err(error) => self.journal.warning(APP, format!("{file}: the settings view did not save: {error}")),
        }
        Arc::make_mut(&mut self.state().config_file).save = Some(SaveState { number, result: Some(written) });
        self.repaint.request();
        Ok(())
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
        if config.set(Setting::JournalEntries).is_some() {
            self.journal.set_limit(usize::try_from(config.number(Setting::JournalEntries)).unwrap_or(usize::MAX));
        }
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
