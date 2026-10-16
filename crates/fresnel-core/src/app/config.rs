//! The settings file in the core (plan section 12.5): the read at the start, a read again
//! after a change of the file, and the saves of the settings view.
//!
//! The core reads `config.toml` before the login shell, so the keys of the user act from
//! the first frames. Each problem of the file goes to the journal with its line and column.
//! The views read the settings in `CoreView::config` and act on a change at once. The core
//! builds its connections, stores and views with the settings of the first read: a change of
//! a group of `Group::at_start` acts at the next start, and the journal says so. The result of
//! a save comes with the read of the file after it, so a view that shows "saved" shows the
//! settings of the file too, never those of before.

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
        let config = self.read_config(None).await?;
        self.effective.get_or_init(|| apply(&config, &self.settings, self.home().as_deref()));
        self.state().config_read = true;
        Ok(())
    }

    /// Reads the settings file again after a change of it. The settings of the views act
    /// at once, together with `save`, the result of a save of the settings view.
    pub(super) async fn reload_config(&self, save: Option<SaveState>) -> Result<(), CoreError> {
        let config = self.read_config(save).await?;
        let wanted = apply(&config, &self.settings, self.home().as_deref());
        // The settings of the core have no equality of their own, and their text has each field.
        if format!("{wanted:?}") != format!("{:?}", self.settings()) {
            let text = "the network, credential plugin, metrics, log and Events settings act at the next start";
            self.journal.info(APP, format!("{}: {text}", self.config_file.display()));
        }
        Ok(())
    }

    /// Writes the changes of the settings view into the file and reads it and the kubeconfig
    /// files again. The result of the write shows in the same change of the state as the
    /// settings of that read; a read that fails shows it alone, as the view waits for one.
    pub(super) async fn save_config(&self, registry: &Registry, edits: Vec<Edit>) -> Result<(), CoreError> {
        let save = self.write_config(edits).await;
        let reloaded = self.reload_files(registry, Some(save.clone())).await;
        if reloaded.is_err() {
            Arc::make_mut(&mut self.state().config_file).save = Some(save);
            self.repaint.request();
        }
        reloaded
    }

    /// Writes the changes of the settings view into the file. Gives the result of the save,
    /// which the views do not see yet: the file has the new settings, the views not.
    async fn write_config(&self, edits: Vec<Edit>) -> SaveState {
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
        SaveState { number, result: Some(written) }
    }

    /// Reads the settings file and the kubeconfig files again, after a change of one of
    /// them or with `save`, a save of the settings view. The caller then connects the
    /// clusters of the selection.
    pub(super) async fn reload_files(&self, registry: &Registry, save: Option<SaveState>) -> Result<(), CoreError> {
        // The stamps come before the reads: a change during the reads then gives one more read.
        self.mark_files().await?;
        self.reload_config(save).await?;
        let env = self.state().env.clone();
        if let Some(env) = env {
            self.load_kubeconfig(&env, registry).await?;
        }
        Ok(())
    }

    /// Reads the file, gives its settings to the views and each problem to the journal.
    /// Gives the settings in use: those of the file, or the defaults for a file that the app
    /// does not use.
    async fn read_config(&self, save: Option<SaveState>) -> Result<Config, CoreError> {
        let path = self.config_file.clone();
        let loaded = self.off_worker(move || config::load(&path)).await?;
        let file = self.config_file.display();
        let (read, settings) = match loaded {
            Ok(None) => {
                self.journal.info(APP, format!("{file}: no file; each setting has its default"));
                (FileRead::Absent, Some(Config::default()))
            }
            Ok(Some(loaded)) => self.report(loaded),
            Err(error) => {
                self.journal.warning(APP, format!("{file}: {error}; {}", self.kept()));
                (FileRead::Failed(error.to_string()), None)
            }
        };
        // One change of the state: a view sees the settings, the read and the save together.
        let config = {
            let mut state = self.state();
            if let Some(settings) = settings {
                state.config = Arc::new(settings);
            }
            let file_state = Arc::make_mut(&mut state.config_file);
            file_state.read = read;
            if let Some(save) = save {
                file_state.save = Some(save);
            }
            Arc::clone(&state.config)
        };
        // The limits act at once. A file with no limit gives the limit of the program again.
        let limit = config.set(Setting::JournalEntries).map(|_| config.number(Setting::JournalEntries));
        self.journal.set_limit(limit.map(|limit| usize::try_from(limit).unwrap_or(usize::MAX)));
        let repaints = config.set(Setting::RepaintLimit).map(|_| config.number(Setting::RepaintLimit));
        let repaints = repaints.map_or(self.settings.repaints_per_second, |n| u32::try_from(n).unwrap_or(u32::MAX));
        self.repaint.set_limit(repaints);
        self.follow_terminal_settings(&config);
        self.repaint.request();
        Ok(Config::clone(&config))
    }

    /// Gives each problem of a file to the journal, and the settings of the file for the
    /// views. A file that the app does not use gives none, so the settings stay as they are
    /// (design rule 5): a mistake in an editor must not make a read-only cluster writable.
    fn report(&self, loaded: Loaded) -> (FileRead, Option<Config>) {
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
        (FileRead::Read { problems, used }, used.then_some(config))
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
        self.base_env.home().map(PathBuf::from)
    }
}

#[cfg(test)]
mod tests {
    use fresnel_net::WallClock;
    use fresnel_testkit::Home;
    use tokio::runtime::Handle;

    use super::super::{Command, Core, CoreParts, CoreSettings, Startup};
    use crate::config::{Edit, EditValue, Theme};
    use crate::journal::Journal;

    fn core_of(home: &Home) -> Core {
        Core::new(CoreParts {
            runtime: Handle::current(),
            base_env: home.finder_env(),
            journal: Journal::new(2000, WallClock::system()),
            state_file: home.path("state/state.json"),
            config_file: home.path("state/config.toml"),
            downloads: home.path("Downloads"),
            detectors: Vec::new(),
            settings: CoreSettings::default(),
        })
    }

    /// The view with the result of a save of the settings view has the settings of that save.
    /// Before, the core showed the result and then read the file, so a view in that time
    /// showed "saved" with the old settings, and the settings view, which empties its draft
    /// at the result, showed the old values. The test takes a view after each step of the
    /// command, at each wait for a file: each state that a frame could see.
    #[tokio::test]
    async fn the_view_with_the_result_of_a_save_has_the_settings_of_the_save() {
        let home = Home::new("save-order", &["kubeconfig.yaml"]).expect("a home folder");
        home.write("work/kubeconfig.yaml", "contexts:\n- {name: a, context: {cluster: none}}\n").expect("a file");
        home.write("state/config.toml", "version = 1\n[appearance]\ntheme = \"dark\"\n").expect("the file");
        let core = core_of(&home);
        core.start(|| {});
        while core.view().startup != Startup::Ready {
            tokio::task::yield_now().await;
        }
        assert_eq!(core.view().config.theme(), Theme::Dark);
        let registry = core.inner.registry.get().cloned().expect("the registry of the start");

        let light = Edit::new(&["appearance", "theme"], EditValue::Text("light".to_owned()));
        let mut save = std::pin::pin!(core.inner.apply(&registry, Command::EditSettings(vec![light])));
        let mut views = Vec::new();
        let saved = std::future::poll_fn(|context| {
            let step = save.as_mut().poll(context);
            let view = core.view();
            views.push((view.config_file.save.as_ref().and_then(|save| save.result.clone()), view.config.theme()));
            step
        })
        .await;
        core.stop();
        saved.expect("the save");
        assert!(views.len() > 2, "a view at each wait for a file: {views:?}");
        let results: Vec<_> = views.iter().filter(|(result, _)| result.is_some()).collect();
        assert!(!results.is_empty() && results.iter().all(|view| **view == (Some(Ok(())), Theme::Light)), "{views:?}");
    }

    /// The limits of the journal and of the repaints in the file act at once, and a file with
    /// no limit gives the limit of the program again.
    #[tokio::test]
    async fn the_limits_of_the_file_act_at_once_and_after_them_the_limits_of_the_program() {
        let home = Home::new("limits", &[]).expect("a home folder");
        let text = "version = 1\n[diagnostics]\njournal_entries = 1000\n[appearance]\nrepaints_per_second = 10\n";
        home.write("state/config.toml", text).expect("the file");
        let core = core_of(&home);
        let journal = core.inner.journal.clone();
        let fill = || (0..3000).for_each(|number| journal.info("test", number.to_string()));
        core.inner.load_config().await.expect("the read");
        fill();
        assert_eq!((journal.view().entries.len(), core.inner.repaint.limit()), (1000, 10), "the limits of the file");
        home.write("state/config.toml", "version = 1\n").expect("a file with no limit");
        core.inner.reload_config(None).await.expect("the read");
        fill();
        assert_eq!((journal.view().entries.len(), core.inner.repaint.limit()), (2000, 60), "of the program");
    }
}
