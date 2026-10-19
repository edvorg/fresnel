//! The reads and the saves of the editors (plan section 12.9): the text of each editor that
//! the UI shows, and its last save.
//!
//! One task reads the object of each new editor and runs each save, all at the same time: a
//! cluster that does not answer holds no editor of an other cluster. A read runs once; the
//! text then belongs to the editor, and a later version of the object does not change it.
//! An editor that closes drops its read; a save of it runs to its answer, which then goes to
//! the journal (design rule 6).
//!
//! - `save`: one save: its dry run, its replace and the merge after a conflict.

mod save;

use std::collections::BTreeSet;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use fresnel_net::ErrorClass;
use fresnel_net::task::{Backoff, Restart, TaskHandle, TaskName, Tasks};
use futures_util::StreamExt as _;
use futures_util::stream::FuturesUnordered;
use tokio::sync::{Notify, watch};

pub use save::{EditSave, Remote, SaveState, SaveView, Stage};

use super::repaint::Repaint;
use super::{Inner, Phase};
use crate::cluster::Registry;
use crate::error::CoreError;
use crate::journal::{Journal, Level};
use crate::manifest::{self, EditText};
use crate::ops::Target;

/// The most editors whose texts the core reads: each one holds a text of up to 8 MiB, and a
/// save up to three. The view opens no more; the core reads only the first ones of a longer list.
pub const EDITOR_LIMIT: usize = 8;

/// The text that an editor asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditRequest {
    /// The identity of the request, which the view gives: a new one reads again.
    pub number: u64,
    /// The object of the editor.
    pub target: Target,
}

/// The read of the text of one editor, and its last save.
#[derive(Clone, Debug)]
pub struct EditRead {
    pub request: EditRequest,
    pub state: EditState,
    pub save: Option<SaveView>,
}

/// How far the read of an editor is.
#[derive(Clone, Debug)]
pub enum EditState {
    /// The request is on its way, with the time limit of a GET.
    Reading,
    Done(Arc<EditText>),
    Failed(CoreError),
}

impl Inner {
    /// Reads the text of each editor that the UI shows and has no text yet. The task of the
    /// reads starts with the first editor.
    pub(super) fn show_editors(&self, registry: &Registry, requests: Vec<EditRequest>) {
        let mut state = self.state();
        if state.phase == Phase::Stopped {
            return;
        }
        let dropped = state.edits.get_or_insert_with(|| self.edit_reader(registry)).show(requests);
        drop(state);
        if dropped > 0 {
            let asked = dropped + EDITOR_LIMIT;
            let text = format!("a view asked for {asked} editors: the core reads the first {EDITOR_LIMIT}");
            self.journal.warning(super::APP, text);
        }
        self.repaint.request();
    }

    /// Gives a save to the task of its editor. A read-only cluster refuses it, also from a
    /// view that did not close its gate: the view shows that as the state of the save.
    pub(super) fn save_editor(&self, registry: &Registry, save: EditSave) {
        let mut state = self.state();
        if state.phase == Phase::Stopped {
            return;
        }
        let refusal = state.config.refusal(&save.editor.target.cluster);
        let reader = state.edits.get_or_insert_with(|| self.edit_reader(registry));
        let shown = match &refusal {
            Some(refusal) => {
                reader.give(&save, SaveState::NotSent(CoreError::new(ErrorClass::Forbidden, refusal.clone())))
            }
            None => reader.give(&save, SaveState::Pending),
        };
        drop(state);
        let why = match (shown, refusal) {
            (_, Some(refusal)) => Some(refusal),
            (false, None) => Some("its editor is not open".to_owned()),
            (true, None) => None,
        };
        if let Some(why) = why {
            let text = format!("{} did not go: {why}", save::save_of(&save));
            self.journal.warning(save.editor.target.cluster.as_str(), text);
        }
        self.repaint.request();
    }

    fn edit_reader(&self, registry: &Registry) -> EditReader {
        EditReader::start(&self.tasks, registry.clone(), (self.repaint.clone(), self.journal.clone()))
    }
}

/// The reads of the editors, in the sequence of their requests.
type Reads = Arc<Mutex<Vec<Arc<EditRead>>>>;

fn lock(reads: &Reads) -> MutexGuard<'_, Vec<Arc<EditRead>>> {
    reads.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The task that reads the texts of the editors and runs their saves. It stops when this
/// value drops.
pub(super) struct EditReader {
    _task: TaskHandle,
    wanted: watch::Sender<Vec<EditRequest>>,
    reads: Reads,
    saved: Arc<Notify>,
}

/// What the task needs besides its reads.
#[derive(Clone)]
struct Parts {
    registry: Registry,
    repaint: Repaint,
    /// Takes the end of a save that no editor shows.
    journal: Journal,
    saved: Arc<Notify>,
}

impl EditReader {
    fn start(tasks: &Tasks, registry: Registry, (repaint, journal): (Repaint, Journal)) -> Self {
        let wanted = watch::Sender::new(Vec::new());
        let (reads, saved) = (Reads::default(), Arc::new(Notify::new()));
        let parts = Parts { registry, repaint, journal, saved: Arc::clone(&saved) };
        let mut runs = 0_u64;
        let task = tasks.start(TaskName::new("editor reads"), Restart::Always(Backoff::default()), {
            let (wanted, reads) = (wanted.clone(), Arc::clone(&reads));
            move || {
                runs += 1;
                if runs > 1 {
                    end_after_restart(&reads);
                }
                serve(wanted.subscribe(), Arc::clone(&reads), parts.clone())
            }
        });
        Self { _task: task, wanted, reads, saved }
    }

    /// Gives a save to the read of its editor in this state: pending for the task, or a
    /// refusal. The same save again while it is pending adds nothing, as from a second
    /// click. False when no read has the editor of the save.
    fn give(&self, save: &EditSave, state: SaveState) -> bool {
        let mut reads = lock(&self.reads);
        let Some(read) = reads.iter_mut().find(|read| read.request == save.editor) else {
            return false;
        };
        let pending = read.save.as_ref().is_some_and(|view| view.state == SaveState::Pending);
        if pending && (view_number(read) == Some(save.number) || state != SaveState::Pending) {
            return true;
        }
        let view = SaveView { save: save.clone(), state };
        *read = Arc::new(EditRead { save: Some(view), ..EditRead::clone(read) });
        drop(reads);
        self.saved.notify_one();
        true
    }

    /// Keeps the read of each of these requests, to the limit, and drops each other one.
    /// Gives the count of the requests past the limit.
    fn show(&self, mut requests: Vec<EditRequest>) -> usize {
        let dropped = requests.len().saturating_sub(EDITOR_LIMIT);
        requests.truncate(EDITOR_LIMIT);
        {
            let mut reads = lock(&self.reads);
            let kept: Vec<Arc<EditRead>> = requests
                .iter()
                .map(|request| {
                    let read = reads.iter().find(|read| read.request == *request);
                    read.map_or_else(
                        || Arc::new(EditRead { request: request.clone(), state: EditState::Reading, save: None }),
                        Arc::clone,
                    )
                })
                .collect();
            *reads = kept;
        }
        self.wanted.send_replace(requests);
        dropped
    }

    /// The reads, for the view.
    pub(super) fn reads(&self) -> Vec<Arc<EditRead>> {
        lock(&self.reads).clone()
    }
}

/// Ends each replace that waits or runs as a write whose result is not known: the run of the
/// task before this one can have ended during it, and it can be in effect, so it does not go
/// again (design rule 6). A dry run goes again: it stores nothing.
fn end_after_restart(reads: &Reads) {
    let mut kept = lock(reads);
    for read in kept.iter_mut() {
        let Some(view) = read.save.as_ref().filter(|view| view.state == SaveState::Pending) else {
            continue;
        };
        if view.save.stage == Stage::Send {
            let error = CoreError::invalid("the task of the editors started again during the save");
            let view = SaveView { save: view.save.clone(), state: SaveState::Unknown(error, None) };
            *read = Arc::new(EditRead { save: Some(view), ..EditRead::clone(read) });
        }
    }
}

/// The number of the save of a read.
fn view_number(read: &EditRead) -> Option<u64> {
    read.save.as_ref().map(|view| view.save.number)
}

/// The answer of a read or of a save.
enum Answer {
    Read(EditRequest, EditState),
    Save(EditSave, SaveState),
}

/// A read or a save of one editor in progress, with its answer.
type Pending = Pin<Box<dyn Future<Output = Answer> + Send>>;

/// The body of the task: starts a read for each request that has no text and each save that
/// waits, and keeps each answer of a request that the UI still shows.
async fn serve(mut wanted: watch::Receiver<Vec<EditRequest>>, reads: Reads, parts: Parts) -> Result<(), CoreError> {
    let mut started: Vec<EditRequest> = Vec::new();
    // The saves that run: the request of the editor and the number of the save.
    let mut saving: BTreeSet<(u64, u64)> = BTreeSet::new();
    let mut pending: FuturesUnordered<Pending> = FuturesUnordered::new();
    loop {
        let requests = wanted.borrow_and_update().clone();
        started.retain(|request| requests.contains(request));
        for request in requests {
            let waits =
                lock(&reads).iter().any(|read| read.request == request && matches!(read.state, EditState::Reading));
            if waits && !started.contains(&request) {
                started.push(request.clone());
                let registry = parts.registry.clone();
                pending.push(Box::pin(async move { read(registry, request).await }));
            }
        }
        let waiting: Vec<EditSave> = lock(&reads)
            .iter()
            .filter_map(|read| read.save.as_ref().filter(|view| view.state == SaveState::Pending))
            .map(|view| view.save.clone())
            .collect();
        for edit in waiting {
            if saving.insert((edit.editor.number, edit.number)) {
                let registry = parts.registry.clone();
                pending.push(Box::pin(async move {
                    let (save, state) = save::run(registry, edit).await;
                    Answer::Save(save, state)
                }));
            }
        }
        tokio::select! {
            changed = wanted.changed() => changed.map_err(|_| CoreError::invalid("the core stopped"))?,
            () = parts.saved.notified() => {}
            Some(answer) = pending.next() => {
                if let Answer::Save(save, _) = &answer {
                    saving.remove(&(save.editor.number, save.number));
                }
                keep(&reads, &parts.journal, answer);
                parts.repaint.request();
            }
        }
    }
}

/// Keeps an answer in the read of its editor. The end of a replace that no editor shows any
/// more goes to the journal, with no value.
fn keep(reads: &Reads, journal: &Journal, answer: Answer) {
    let mut kept = lock(reads);
    match answer {
        Answer::Read(request, state) => {
            if let Some(slot) = kept.iter_mut().find(|read| read.request == request) {
                *slot = Arc::new(EditRead { request, state, save: slot.save.clone() });
            }
        }
        Answer::Save(save, state) => {
            let slot =
                kept.iter_mut().find(|read| read.request == save.editor && view_number(read) == Some(save.number));
            match slot {
                Some(slot) => {
                    *slot = Arc::new(EditRead { save: Some(SaveView { save, state }), ..EditRead::clone(slot) })
                }
                None if save.stage == Stage::Send => {
                    drop(kept);
                    let level = if matches!(state, SaveState::Saved(_)) { Level::Info } else { Level::Warning };
                    let text = format!("{} ended after its editor closed: {state}", save::save_of(&save));
                    journal.push(level, save.editor.target.cluster.as_str(), text);
                }
                None => {}
            }
        }
    }
}

/// Reads the text of one editor through the gateway of its cluster.
async fn read(registry: Registry, request: EditRequest) -> Answer {
    let Target { cluster, kind, key, uid } = &request.target;
    let state = match registry.link(cluster) {
        None => EditState::Failed(CoreError::invalid(format!("{cluster} is not connected"))),
        Some(link) => match manifest::read_for_edit(&link.gateway, *kind, key, uid).await {
            Ok(text) => EditState::Done(Arc::new(text)),
            Err(error) => EditState::Failed(error),
        },
    };
    Answer::Read(request, state)
}

#[cfg(test)]
mod tests;
