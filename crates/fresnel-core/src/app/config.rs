//! The read of the settings file at the start of the core (plan section 12.5).
//!
//! The core reads `config.toml` one time, before the login shell, so the keys of the
//! user act from the first frames. Each problem of the file goes to the journal with its
//! line and column. The views read the settings in `CoreView::config`.

use std::sync::Arc;

use super::{APP, Inner};
use crate::config::{self, Loaded};
use crate::error::CoreError;

impl Inner {
    /// Reads the settings file, if no run of the task read it before.
    pub(super) async fn load_config(&self) -> Result<(), CoreError> {
        if self.state().config_read {
            return Ok(());
        }
        let path = self.config_file.clone();
        let loaded = self.off_worker(move || config::load(&path)).await?;
        let file = self.config_file.display();
        match loaded {
            Ok(None) => self.journal.info(APP, format!("{file}: no file; each setting has its default")),
            Ok(Some(loaded)) => self.report(loaded),
            Err(error) => self.journal.warning(APP, format!("{file}: {error}; each setting has its default")),
        }
        self.state().config_read = true;
        self.repaint.request();
        Ok(())
    }

    /// Gives the settings of a file to the views, and each problem of it to the journal.
    fn report(&self, loaded: Loaded) {
        let file = self.config_file.display();
        let Loaded { config, problems, used } = loaded;
        for problem in &problems {
            if used {
                self.journal.warning(APP, format!("{file}: {problem}"));
            } else {
                self.journal.error(APP, format!("{file}: {problem}; the app uses no setting of the file"));
            }
        }
        if used {
            if problems.is_empty() {
                self.journal.info(APP, format!("{file}: read"));
            }
            self.state().config = Arc::new(config);
        }
    }
}
