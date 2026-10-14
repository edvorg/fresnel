//! The selection of clusters and the table layouts in the state file (plan section 12.5).
//!
//! The core reads the file one time at its start, and writes it after a command of the
//! user changed the selection or a layout. Each read and each write runs on a thread for
//! blocking calls.

use std::collections::BTreeSet;
use std::sync::Arc;

use super::{APP, Inner, Phase};
use crate::cluster::{ClusterId, Registry};
use crate::error::CoreError;
use crate::state_file::{self, SavedState, TableLayout};

/// What the state file has of the state of the core.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Saved {
    /// The file has the state, or the user did not change it yet.
    Current,
    /// A command of the user changed the state after the last write.
    Changed,
    /// The file did not load at the start. The core does not write over it.
    Blocked,
}

impl Inner {
    /// Reads the state file. `None` with no file, and for a file that the app cannot use:
    /// that file stays as it is, and the journal tells why.
    pub(super) async fn load_saved_state(&self) -> Result<Option<SavedState>, CoreError> {
        let path = self.state_file.clone();
        match self.off_worker(move || state_file::load(&path)).await? {
            Ok(saved) => Ok(saved),
            Err(error) => {
                let file = self.state_file.display();
                let note = "the app does not write over this file, so it does not save the state of this run";
                self.journal.warning(APP, format!("{file}: {error}; {note}"));
                self.state().saved = Saved::Blocked;
                Ok(None)
            }
        }
    }

    /// Sets the state of the last run at the start. With no state file, the selection is
    /// the cluster of `kubectl`, and the user selects the others.
    pub(super) fn restore_at_start(&self, saved: Option<SavedState>, current: Option<String>, registry: &Registry) {
        let Some(saved) = saved else {
            let mut state = self.state();
            state.selected.extend(current.map(ClusterId::new));
            state.tables = Some(Arc::default());
            return;
        };
        let known = known_clusters(registry);
        let (selected, absent): (BTreeSet<ClusterId>, BTreeSet<ClusterId>) =
            saved.selected.iter().cloned().partition(|id| known.contains(id));
        self.journal.info(APP, format!("the selection of the last run: {}", names(&selected)));
        if !absent.is_empty() {
            let note = "they stay in the state file";
            let text =
                format!("no kubeconfig file has these clusters of the last selection: {}; {note}", names(&absent));
            self.journal.warning(APP, text);
        }
        let mut state = self.state();
        state.selected.extend(selected);
        state.in_state_file = saved.selected;
        state.tables = Some(Arc::new(saved.tables));
    }

    /// Selects a cluster or removes it from the selection, for a command of the user.
    pub(super) fn select(&self, cluster: ClusterId, selected: bool) {
        let mut state = self.state();
        let changed = if selected { state.selected.insert(cluster) } else { state.selected.remove(&cluster) };
        if changed {
            state.changed();
        }
    }

    /// Keeps the layout of a table for the next write, for a command of the user. The view
    /// gets the new layouts too. A command runs after the start, which read the file.
    pub(super) fn save_table(&self, table: String, layout: TableLayout) {
        let mut state = self.state();
        let Some(tables) = state.tables.as_mut().map(Arc::make_mut) else {
            return;
        };
        let changed = if layout == TableLayout::default() {
            tables.remove(&table).is_some()
        } else {
            tables.insert(table, layout.clone()) != Some(layout)
        };
        if changed {
            state.changed();
            drop(state);
            self.repaint.request();
        }
    }

    /// Writes the selection and the table layouts to the state file, if a command changed
    /// them after the last write. A write that fails gives a journal entry, and the next
    /// change tries again.
    ///
    /// A cluster of the file that no kubeconfig file has now stays in the file. A kubeconfig
    /// file can be absent for one run, and the selection of its clusters must not go with it.
    pub(super) async fn save_state(&self, registry: &Registry) -> Result<(), CoreError> {
        let known = known_clusters(registry);
        let saved = {
            let mut state = self.state();
            // After the stop, the registry is empty, and a write would take each cluster
            // for one that no kubeconfig file has. Before the read of the file, a write
            // would lose its tables.
            let Some(tables) = state.tables.clone() else {
                return Ok(());
            };
            if state.saved != Saved::Changed || state.phase == Phase::Stopped {
                return Ok(());
            }
            state.saved = Saved::Current;
            let absent = state.in_state_file.iter().filter(|id| !known.contains(*id)).cloned();
            let selected: BTreeSet<ClusterId> = state.selected.iter().cloned().chain(absent).collect();
            state.in_state_file.clone_from(&selected);
            SavedState { selected, tables: Arc::unwrap_or_clone(tables) }
        };
        let path = self.state_file.clone();
        if let Err(error) = self.off_worker(move || state_file::save(&path, &saved)).await? {
            let file = self.state_file.display();
            self.journal.warning(APP, format!("the state is not saved: {file}: {error}"));
            // The repaint of the change came before the write, so the warning asks for its own.
            self.repaint.request();
        }
        Ok(())
    }
}

impl super::State {
    /// Marks a change of the user for the next write. A file that did not load stays blocked.
    fn changed(&mut self) {
        if self.saved == Saved::Current {
            self.saved = Saved::Changed;
        }
    }
}

fn known_clusters(registry: &Registry) -> BTreeSet<ClusterId> {
    registry.snapshot().into_iter().map(|cluster| cluster.id).collect()
}

/// The names of some clusters as one text.
fn names(clusters: &BTreeSet<ClusterId>) -> String {
    if clusters.is_empty() {
        return "no cluster".to_owned();
    }
    clusters.iter().map(ClusterId::as_str).collect::<Vec<_>>().join(", ")
}
